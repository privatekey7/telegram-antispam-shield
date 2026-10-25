//! Spam reports for admins, with buttons to undo or confirm.
//!
//! Every automatic deletion is reported to `ANTISPAM_REPORT_CHAT` (or, if
//! unset, to `ANTISPAM_OWNER_ID` in private). The report shows who, why and
//! what, with buttons:
//!
//! * "Not spam": unban or unmute the sender, trust them, and learn the text
//!   as not spam, so similar messages pass from now on;
//! * "Ban" (when the action only deletes): ban, clean up, and learn the
//!   text as spam.
//!
//! Only the owner or an admin of the reported chat may press them: "Not spam"
//! unbans, so it needs the "Ban users" right; "Ban" also deletes, so it needs
//! "Delete messages" as well.
//!
//! Reports come only from chats that pass `ANTISPAM_ALLOWED_CHATS` (every
//! chat when it is unset), and each chat gets at most [`REPORTS_PER_WINDOW`]
//! reports per [`REPORT_WINDOW`], so one noisy (or hostile) chat cannot flood
//! the owner. Many chats still can; only the allowlist stops that.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use teloxide::prelude::*;
use teloxide::types::{
    CallbackQuery, ChatId, ChatPermissions, InlineKeyboardButton, InlineKeyboardMarkup,
    LinkPreviewOptions, UserId,
};

use telegram_antispam_shield::access::{Access, AdminAction};
use telegram_antispam_shield::config::Action;
use telegram_antispam_shield::moderation::Reason;
use telegram_antispam_shield::shield::ReportKey;
use telegram_antispam_shield::store::ReportRecord;
use telegram_antispam_shield::text;

use super::actions::{self, Sender};
use super::{rights, AppState};

/// Text kept from the reported message (the report must stay readable).
const REPORT_TEXT_CHARS: usize = 800;
/// Reports one chat may send within [`REPORT_WINDOW`]; the rest are only logged.
const REPORTS_PER_WINDOW: u32 = 20;
const REPORT_WINDOW: Duration = Duration::from_secs(10 * 60);

/// Per-chat report counts in fixed windows.
#[derive(Default)]
pub struct ReportBudget {
    chats: HashMap<i64, (Instant, u32)>,
}

/// Whether a report may be sent.
#[derive(Debug, PartialEq, Eq)]
enum Spend {
    Granted,
    /// The first report over the limit in this window (worth one log line).
    JustExhausted,
    Refused,
}

impl ReportBudget {
    fn spend(&mut self, chat: i64, now: Instant) -> Spend {
        let (started, count) = self.chats.entry(chat).or_insert((now, 0));
        if now.duration_since(*started) >= REPORT_WINDOW {
            (*started, *count) = (now, 0);
        }
        *count = count.saturating_add(1);
        match *count {
            n if n <= REPORTS_PER_WINDOW => Spend::Granted,
            n if n == REPORTS_PER_WINDOW + 1 => Spend::JustExhausted,
            _ => Spend::Refused,
        }
    }
}

/// Names and chat titles in a report are cut to this many characters.
const REPORT_NAME_CHARS: usize = 64;
/// Prefix of every line of the reported text, so text from the sender can
/// never pass for the bot's own words.
const QUOTE_PREFIX: &str = "│ ";

/// What happened to one spam message, for the report.
pub struct Incident<'a> {
    pub chat: &'a teloxide::types::Chat,
    pub sender: &'a Sender<'a>,
    pub text: &'a str,
    pub reason: Reason,
    pub score: f32,
    pub deleted: usize,
    /// The spam was edited into an earlier message.
    pub edited: bool,
}

/// Send a report about `incident`, if reports are configured.
pub async fn send(bot: &Bot, state: &AppState, incident: Incident<'_>) {
    let Some(destination) = state.settings.report_destination() else {
        return;
    };
    let chat_id = incident.chat.id.0;
    match state.report_budget().spend(chat_id, Instant::now()) {
        Spend::Granted => {}
        Spend::JustExhausted => {
            tracing::warn!(
                chat = chat_id,
                limit = REPORTS_PER_WINDOW,
                "report_limit_reached"
            );
            return;
        }
        Spend::Refused => return,
    }
    let text: String = incident.text.chars().take(REPORT_TEXT_CHARS).collect();
    let key = state.shield().save_report(ReportRecord {
        chat_id: incident.chat.id.0,
        sender_id: incident.sender.id(),
        text: text.clone(),
    });

    let action = state.settings.action;
    let outcome = match action {
        Action::DeleteOnly => "удалено · deleted",
        Action::DeleteAndBan => "удалено, бан · deleted, banned",
        Action::DeleteAndMute => "удалено, мут · deleted, muted",
    };
    let chat_name = shorten(incident.chat.title().unwrap_or_default());
    let title = if incident.edited {
        "🛡 Спам (правка) · Spam (edited)"
    } else {
        "🛡 Спам · Spam"
    };
    // Plain text (no parse mode), so names and spam cannot inject markup.
    // Everything the bot did not write itself is marked: the chat title and
    // the sender's name in «», the message line by line behind a bar.
    let body = format!(
        "{title}\n\
         Чат · Chat: «{chat_name}» (id {})\n\
         Автор · Sender: «{}» (id {})\n\
         Причина · Reason: {} ({:.2})\n\
         Итог · Outcome: {outcome}, {} msg\n\
         Текст отправителя, не бота · Sender's text, not the bot's:\n{}",
        incident.chat.id,
        shorten(&incident.sender.display_name()),
        incident.sender.id(),
        incident.reason.as_str(),
        incident.score,
        incident.deleted,
        quote(&text),
    );

    let mut row = vec![InlineKeyboardButton::callback(
        "✅ Не спам · Not spam",
        callback_data("ham", key),
    )];
    if action == Action::DeleteOnly {
        row.push(InlineKeyboardButton::callback(
            "🚫 Бан · Ban",
            callback_data("ban", key),
        ));
    }
    let sent = bot
        .send_message(ChatId(destination), body)
        .link_preview_options(no_preview())
        .reply_markup(InlineKeyboardMarkup::new([row]))
        .await;
    if let Err(e) = sent {
        // A private report fails until the owner has pressed Start.
        tracing::warn!(error = %e, "report_failed (owner must /start the bot)");
    }
}

/// A press on a report button.
pub async fn on_callback(
    bot: Bot,
    query: CallbackQuery,
    state: std::sync::Arc<AppState>,
) -> ResponseResult<()> {
    let parsed = query.data.as_deref().and_then(parse_callback);
    let Some((verb, key)) = parsed else {
        return Ok(());
    };
    let id = key.id;
    let report = state.shield().report(key);
    let Some(report) = report else {
        bot.answer_callback_query(query.id)
            .text("Отчёт устарел · Report expired")
            .await?;
        return Ok(());
    };

    let chat = ChatId(report.chat_id);
    let presser = query.from.id;
    let action = match verb {
        Verb::NotSpam => AdminAction::NotSpam,
        Verb::Ban => AdminAction::ConfirmBan,
    };
    // A button press always comes from a person, never from the group itself.
    let access = rights::authorize(&bot, &state, chat, Some(&query.from), false, action).await;
    if access != Access::Allowed {
        tracing::info!(report = id, admin = presser.0, access = ?access, "report_action_refused");
        bot.answer_callback_query(query.id)
            .text(rights::refusal_text(access))
            .await?;
        return Ok(());
    }

    let verdict = match verb {
        Verb::NotSpam => {
            state
                .shield()
                .vouch(report.chat_id, report.sender_id, &report.text);
            pardon(&bot, &state, chat, report.sender_id).await;
            "✅ Не спам: разбанен, доверие выдано · Not spam: unbanned, trusted"
        }
        Verb::Ban => {
            let recent = {
                let mut shield = state.shield();
                shield.learn_spam(report.chat_id, report.sender_id, &report.text);
                shield.recent_from(report.chat_id, report.sender_id)
            };
            actions::delete_all(&bot, chat, recent).await;
            ban(&bot, chat, report.sender_id).await;
            "🚫 Забанен · Banned"
        }
    };
    tracing::info!(report = id, admin = presser.0, verb = ?verb, "report_action");
    bot.answer_callback_query(query.id.clone())
        .text(verdict)
        .await?;

    // Mark the report as handled and drop its buttons.
    if let Some(message) = query.regular_message() {
        let by = query
            .from
            .mention()
            .unwrap_or_else(|| query.from.full_name());
        let text = format!("{}\n\n{verdict} — {by}", message.text().unwrap_or_default());
        if let Err(e) = bot
            .edit_message_text(message.chat.id, message.id, text)
            .link_preview_options(no_preview())
            .await
        {
            tracing::debug!(error = %e, "report_edit_failed");
        }
    }
    Ok(())
}

/// `raw` on one line without direction overrides, cut to [`REPORT_NAME_CHARS`].
fn shorten(raw: &str) -> String {
    let line = text::strip_bidi(raw)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if line.chars().count() <= REPORT_NAME_CHARS {
        return line;
    }
    let short: String = line.chars().take(REPORT_NAME_CHARS - 1).collect();
    format!("{short}…")
}

/// Every line of `raw` behind [`QUOTE_PREFIX`]. Lines break at every
/// character a client may render as a break, not just `\n`, and direction
/// overrides are dropped, so no line can escape the prefix or reorder the
/// bot's own lines.
fn quote(raw: &str) -> String {
    text::strip_bidi(raw)
        .split(is_line_break)
        .map(|line| {
            let line: String = line.chars().filter(|c| !c.is_control()).collect();
            format!("{QUOTE_PREFIX}{line}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_line_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// `verb:<id>:<token>`; at most 4 + 19 + 1 + 16 = 40 bytes of Telegram's 64.
fn callback_data(verb: &str, key: ReportKey) -> String {
    format!("{verb}:{}:{:x}", key.id, key.token)
}

/// Reports quote spam, often with links: never fetch a preview for them.
fn no_preview() -> LinkPreviewOptions {
    LinkPreviewOptions {
        is_disabled: true,
        url: None,
        prefer_small_media: false,
        prefer_large_media: false,
        show_above_text: false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verb {
    NotSpam,
    Ban,
}

/// A button's verb and report. Buttons from before v0.3 carry no token;
/// they match only reports saved before tokens existed (token 0).
fn parse_callback(data: &str) -> Option<(Verb, ReportKey)> {
    let mut parts = data.split(':');
    let verb = match parts.next()? {
        "ham" => Verb::NotSpam,
        "ban" => Verb::Ban,
        _ => return None,
    };
    let id = parts.next()?.parse().ok()?;
    let token = match parts.next() {
        Some(hex) => u64::from_str_radix(hex, 16).ok()?,
        None => 0,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((verb, ReportKey { id, token }))
}

/// Users have positive IDs; channels posting in a group have negative ones.
fn is_channel(sender_id: i64) -> bool {
    sender_id < 0
}

/// Undo a ban or mute of `sender_id`.
async fn pardon(bot: &Bot, state: &AppState, chat: ChatId, sender_id: i64) {
    let result = if is_channel(sender_id) {
        bot.unban_chat_sender_chat(chat, ChatId(sender_id))
            .await
            .map(drop)
    } else if state.settings.action == Action::DeleteAndMute {
        // Back to the group's default permissions.
        let defaults = bot
            .get_chat(chat)
            .await
            .ok()
            .and_then(|full| full.permissions())
            .unwrap_or_else(ChatPermissions::all);
        bot.restrict_chat_member(chat, UserId(sender_id as u64), defaults)
            .await
            .map(drop)
    } else {
        bot.unban_chat_member(chat, UserId(sender_id as u64))
            .only_if_banned(true)
            .await
            .map(drop)
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, "pardon_failed");
    }
}

async fn ban(bot: &Bot, chat: ChatId, sender_id: i64) {
    let result = if is_channel(sender_id) {
        actions::retry_once(|| bot.ban_chat_sender_chat(chat, ChatId(sender_id)).send())
            .await
            .map(drop)
    } else {
        actions::retry_once(|| bot.ban_chat_member(chat, UserId(sender_id as u64)).send())
            .await
            .map(drop)
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, "ban_failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_data_round_trips() {
        // Arrange
        let key = ReportKey {
            id: 42,
            token: u64::MAX,
        };
        let data = callback_data("ban", key);

        // Act
        let parsed = parse_callback(&data);
        let legacy = parse_callback("ham:7");

        // Assert
        assert!(data.len() <= 64, "{data}");
        assert_eq!(parsed, Some((Verb::Ban, key)));
        assert_eq!(legacy, Some((Verb::NotSpam, ReportKey { id: 7, token: 0 })));
        assert_eq!(parse_callback("ham:x"), None);
        assert_eq!(parse_callback("ham:1:zz"), None);
        assert_eq!(parse_callback("ham:1:2:3"), None);
        assert_eq!(parse_callback("other:1"), None);
    }

    #[test]
    fn no_line_of_foreign_text_escapes_the_quote() {
        // Arrange: other line breaks and a right-to-left override.
        let raw = "a\u{2028}fake line\u{2029}b\r\nc\u{85}d \u{202E}tset";

        // Act
        let quoted = quote(raw);

        // Assert
        assert!(
            quoted.lines().all(|l| l.starts_with(QUOTE_PREFIX)),
            "{quoted}"
        );
        assert!(!quoted.contains('\u{202E}'));
    }

    #[test]
    fn report_budget_caps_each_chat_per_window() {
        // Arrange
        let mut budget = ReportBudget::default();
        let now = Instant::now();
        for _ in 0..REPORTS_PER_WINDOW {
            assert_eq!(budget.spend(-1, now), Spend::Granted);
        }

        // Act
        let first_over = budget.spend(-1, now);
        let second_over = budget.spend(-1, now);
        let other_chat = budget.spend(-2, now);

        // Assert
        assert_eq!(first_over, Spend::JustExhausted);
        assert_eq!(second_over, Spend::Refused);
        assert_eq!(other_chat, Spend::Granted);
    }

    #[test]
    fn report_budget_refills_after_the_window() {
        // Arrange
        let mut budget = ReportBudget::default();
        let start = Instant::now();
        for _ in 0..=REPORTS_PER_WINDOW {
            budget.spend(-1, start);
        }

        // Act
        let later = budget.spend(-1, start + REPORT_WINDOW);

        // Assert
        assert_eq!(later, Spend::Granted);
    }

    #[test]
    fn foreign_text_is_quoted_line_by_line() {
        assert_eq!(
            quote("buy now\n✅ Не спам\n\nt.me/x"),
            "│ buy now\n│ ✅ Не спам\n│ \n│ t.me/x"
        );
    }

    #[test]
    fn names_are_one_short_line() {
        assert_eq!(shorten("Group\nname "), "Group name");
        assert_eq!(shorten("\u{202E}evil\u{2066}"), "evil");
        let long = shorten(&"я".repeat(200));
        assert_eq!(long.chars().count(), REPORT_NAME_CHARS);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn channel_ids_are_negative() {
        assert!(is_channel(-1001234567890));
        assert!(!is_channel(42));
    }
}
