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
//! Only the owner or an admin of the reported chat with the "Ban users" right
//! may press them: both buttons ban or unban.
//!
//! Reports come only from allowed chats, and each chat gets at most
//! [`REPORTS_PER_WINDOW`] reports per [`REPORT_WINDOW`], so one noisy (or
//! hostile) chat cannot flood the owner or push other reports out of storage.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use teloxide::prelude::*;
use teloxide::types::{
    CallbackQuery, ChatId, ChatPermissions, InlineKeyboardButton, InlineKeyboardMarkup, UserId,
};

use telegram_antispam_shield::access::{Access, AdminAction};
use telegram_antispam_shield::config::Action;
use telegram_antispam_shield::moderation::Reason;
use telegram_antispam_shield::store::ReportRecord;

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
    let id = state.shield().save_report(ReportRecord {
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
    let chat_name = incident
        .chat
        .title()
        .map_or_else(|| incident.chat.id.to_string(), str::to_owned);
    let title = if incident.edited {
        "🛡 Спам (правка) · Spam (edited)"
    } else {
        "🛡 Спам · Spam"
    };
    // Plain text (no parse mode), so names and spam cannot inject markup.
    let body = format!(
        "{title}\n\
         Чат · Chat: {chat_name}\n\
         Автор · Sender: {} (id {})\n\
         Причина · Reason: {} ({:.2})\n\
         Итог · Outcome: {outcome}, {} msg\n\
         ———\n{text}",
        incident.sender.display_name(),
        incident.sender.id(),
        incident.reason.as_str(),
        incident.score,
        incident.deleted,
    );

    let mut row = vec![InlineKeyboardButton::callback(
        "✅ Не спам · Not spam",
        format!("ham:{id}"),
    )];
    if action == Action::DeleteOnly {
        row.push(InlineKeyboardButton::callback(
            "🚫 Бан · Ban",
            format!("ban:{id}"),
        ));
    }
    let sent = bot
        .send_message(ChatId(destination), body)
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
    let Some((verb, id)) = parsed else {
        return Ok(());
    };
    let report = state.shield().report(id);
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
            .await
        {
            tracing::debug!(error = %e, "report_edit_failed");
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verb {
    NotSpam,
    Ban,
}

fn parse_callback(data: &str) -> Option<(Verb, i64)> {
    let (verb, id) = data.split_once(':')?;
    let verb = match verb {
        "ham" => Verb::NotSpam,
        "ban" => Verb::Ban,
        _ => return None,
    };
    Some((verb, id.parse().ok()?))
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
        bot.ban_chat_sender_chat(chat, ChatId(sender_id))
            .await
            .map(drop)
    } else {
        bot.ban_chat_member(chat, UserId(sender_id as u64))
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
        assert_eq!(parse_callback("ham:42"), Some((Verb::NotSpam, 42)));
        assert_eq!(parse_callback("ban:7"), Some((Verb::Ban, 7)));
        assert_eq!(parse_callback("ham:x"), None);
        assert_eq!(parse_callback("other:1"), None);
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
    fn channel_ids_are_negative() {
        assert!(is_channel(-1001234567890));
        assert!(!is_channel(42));
    }
}
