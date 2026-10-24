//! Carrying out decisions: deleting, banning, muting and flood notices.

use std::future::Future;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use teloxide::errors::AsResponseParameters;
use teloxide::prelude::*;
use teloxide::types::{Chat, ChatId, ChatPermissions, MessageId, User};
use teloxide::RequestError;

use telegram_antispam_shield::config::Action;
use telegram_antispam_shield::flood::FloodReason;

use super::AppState;

/// Telegram's `deleteMessages` accepts at most this many IDs per call.
const DELETE_BATCH_LIMIT: usize = 100;
/// Telegram treats restrictions shorter than 30 s or longer than 366 days as
/// permanent, so mutes are kept inside that range.
const MIN_MUTE: Duration = Duration::from_secs(31);
const MAX_MUTE: Duration = Duration::from_secs(365 * 24 * 60 * 60);
/// Longest `retry_after` from Telegram that is waited out once. Longer waits
/// mean the bot is far over its limits; the action is logged as failed.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(30);
/// Longest name repeated in a group notice, in characters.
const NOTICE_NAME_CHARS: usize = 32;

/// Who posted a message: a person, or a channel posting in the group.
pub enum Sender<'a> {
    User(&'a User),
    Channel(&'a Chat),
}

impl Sender<'_> {
    /// Telegram ID of the user or channel.
    pub fn id(&self) -> i64 {
        match self {
            Sender::User(user) => user.id.0 as i64,
            Sender::Channel(chat) => chat.id.0,
        }
    }

    /// A name that is safe to repeat in the group: the full name (user) or
    /// title (channel) with anything that looks like a link or a mention
    /// removed and cut short, else `id N`. Spammers put ads in their names,
    /// and the bot must not post those for them. No `@username` either: it
    /// can be an ad for a channel or a bot.
    pub fn notice_name(&self) -> String {
        let raw = match self {
            Sender::User(user) => user.full_name(),
            Sender::Channel(chat) => chat.title().unwrap_or_default().to_owned(),
        };
        safe_name(&raw).unwrap_or_else(|| format!("id {}", self.id()))
    }

    /// `@username`, else the full name (user) or title (channel).
    pub fn display_name(&self) -> String {
        match self {
            Sender::User(user) => user.mention().unwrap_or_else(|| user.full_name()),
            Sender::Channel(chat) => chat
                .username()
                .map(|u| format!("@{u}"))
                .or_else(|| chat.title().map(str::to_owned))
                .unwrap_or_else(|| chat.id.to_string()),
        }
    }
}

/// `raw` without words that Telegram could turn into a link or a mention,
/// cut to [`NOTICE_NAME_CHARS`]; `None` when nothing is left.
fn safe_name(raw: &str) -> Option<String> {
    let looks_like_link = |word: &&str| {
        word.chars()
            .any(|c| matches!(c, '@' | '/' | '.' | ':' | '\\') || c.is_control())
    };
    let kept: Vec<&str> = raw
        .split_whitespace()
        .filter(|word| !looks_like_link(word))
        .collect();
    let name = kept.join(" ");
    if name.is_empty() {
        return None;
    }
    if name.chars().count() <= NOTICE_NAME_CHARS {
        return Some(name);
    }
    let short: String = name.chars().take(NOTICE_NAME_CHARS - 1).collect();
    Some(format!("{}…", short.trim_end()))
}

/// The author of a message, or `None` for bots. Posts of the group itself
/// (anonymous admins, the linked channel) are filtered out by the caller.
pub fn sender_of(msg: &Message) -> Option<Sender<'_>> {
    if let Some(chat) = &msg.sender_chat {
        return Some(Sender::Channel(chat));
    }
    let user = msg.from.as_ref()?;
    (!user.is_bot).then_some(Sender::User(user))
}

/// Post `text` in `chat` and delete it after `ttl` so the chat stays clean.
pub async fn send_temporary(bot: &Bot, chat: ChatId, text: String, ttl: Duration) {
    let sent = match bot.send_message(chat, text).await {
        Ok(sent) => sent,
        Err(e) => {
            tracing::warn!(error = %e, "notice_failed");
            return;
        }
    };
    let bot = bot.clone();
    tokio::spawn(async move {
        tokio::time::sleep(ttl).await;
        if let Err(e) = bot.delete_message(chat, sent.id).await {
            tracing::debug!(error = %e, "notice_delete_failed");
        }
    });
}

/// Delete `ids` in as few calls as Telegram allows. Already-deleted messages
/// are fine: Telegram skips them.
pub async fn delete_all(bot: &Bot, chat: ChatId, ids: impl IntoIterator<Item = i32>) {
    let ids: Vec<MessageId> = ids.into_iter().map(MessageId).collect();
    for batch in ids.chunks(DELETE_BATCH_LIMIT) {
        let deleted = retry_once(|| bot.delete_messages(chat, batch.to_vec()).send()).await;
        if let Err(e) = deleted {
            tracing::warn!(error = %e, "delete_failed (is the bot an admin with delete rights?)");
        }
    }
}

/// Apply the configured action to a spammer. Channels cannot be muted, so
/// they are banned whenever the action goes beyond deleting.
pub async fn punish_spammer(bot: &Bot, action: Action, chat: ChatId, sender: &Sender<'_>) {
    let result = match (action, sender) {
        (Action::DeleteOnly, _) => return,
        (Action::DeleteAndBan, Sender::User(user)) => {
            retry_once(|| bot.ban_chat_member(chat, user.id).send())
                .await
                .map(drop)
        }
        (Action::DeleteAndMute, Sender::User(user)) => retry_once(|| {
            bot.restrict_chat_member(chat, user.id, ChatPermissions::empty())
                .send()
        })
        .await
        .map(drop),
        (_, Sender::Channel(channel)) => {
            retry_once(|| bot.ban_chat_sender_chat(chat, channel.id).send())
                .await
                .map(drop)
        }
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, action = ?action, "punish_failed");
    }
}

/// Run `call`, and once more after the wait Telegram asks for when it
/// answers 429 Too Many Requests. During a raid deletes and bans would
/// otherwise be lost silently. Waits longer than [`MAX_RETRY_WAIT`] are not
/// retried.
pub async fn retry_once<T, F, Fut>(mut call: F) -> Result<T, RequestError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, RequestError>>,
{
    match call().await {
        Err(e) => match e.retry_after().map(|s| s.duration()) {
            Some(wait) if wait <= MAX_RETRY_WAIT => {
                tracing::info!(wait_secs = wait.as_secs(), "rate_limited_retrying");
                tokio::time::sleep(wait).await;
                call().await
            }
            _ => Err(e),
        },
        ok => ok,
    }
}

/// What to do with a flooder, as decided by the flood guard.
pub struct FloodAction {
    pub reason: FloodReason,
    pub message_ids: Vec<i32>,
    pub mute_for: Duration,
    pub strike: u32,
}

/// Delete the flood messages, mute the author and optionally tell the chat.
/// Channels cannot be muted, so their flood is only deleted.
pub async fn punish_flood(
    bot: &Bot,
    state: &AppState,
    chat: ChatId,
    sender: &Sender<'_>,
    flood: FloodAction,
) {
    tracing::info!(
        chat = chat.0,
        user = sender.id(),
        reason = flood.reason.as_str(),
        messages = flood.message_ids.len(),
        mute_secs = flood.mute_for.as_secs(),
        strike = flood.strike,
        dry_run = state.settings.dry_run,
        "flood_detected"
    );
    if state.settings.dry_run {
        return;
    }

    delete_all(bot, chat, flood.message_ids).await;

    let Sender::User(user) = sender else {
        return;
    };
    let until = mute_until(flood.mute_for);
    let muted = retry_once(|| {
        bot.restrict_chat_member(chat, user.id, ChatPermissions::empty())
            .until_date(until)
            .send()
    })
    .await;
    if let Err(e) = muted {
        tracing::warn!(error = %e, "flood_mute_failed (grant the bot the \"Ban users\" right)");
        return;
    }

    if state.settings.flood.notify {
        let ttl = state.settings.flood.notify_ttl;
        notify_flood_mute(bot, chat, user, flood.mute_for, ttl).await;
    }
}

/// Post a short mute notice and delete it after `ttl` so the chat stays clean.
async fn notify_flood_mute(
    bot: &Bot,
    chat: ChatId,
    user: &User,
    mute_for: Duration,
    ttl: Duration,
) {
    let who = Sender::User(user).notice_name();
    let length = format_duration(mute_for);
    // Plain text (no parse mode), so user names cannot inject markup.
    let notice = format!("🔇 {who} — мут {length} за флуд · muted {length} for flooding");
    send_temporary(bot, chat, notice, ttl).await;
}

/// Absolute end time of a mute, clamped so Telegram never treats it as permanent.
fn mute_until(mute_for: Duration) -> DateTime<Utc> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let end = now + mute_for.clamp(MIN_MUTE, MAX_MUTE);
    // In range by construction; the epoch fallback is unreachable in practice.
    DateTime::from_timestamp(end.as_secs() as i64, 0).unwrap_or_default()
}

/// Compact, language-neutral duration: `90s`, `10m`, `1h`, `1d`.
fn format_duration(duration: Duration) -> String {
    match duration.as_secs() {
        s if s > 0 && s % 86_400 == 0 => format!("{}d", s / 86_400),
        s if s > 0 && s % 3_600 == 0 => format!("{}h", s / 3_600),
        s if s > 0 && s % 60 == 0 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notice_names_carry_no_links() {
        assert_eq!(safe_name("Иван Петров").as_deref(), Some("Иван Петров"));
        assert_eq!(
            safe_name("BUY → t.me/scam now").as_deref(),
            Some("BUY → now")
        );
        assert_eq!(safe_name("@promo_bot https://x.io").as_deref(), None);
        assert_eq!(safe_name("   ").as_deref(), None);
        let long = safe_name(&"a".repeat(100)).unwrap();
        assert_eq!(long.chars().count(), NOTICE_NAME_CHARS);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn durations_are_formatted_compactly() {
        assert_eq!(format_duration(Duration::from_secs(90)), "90s");
        assert_eq!(format_duration(Duration::from_secs(600)), "10m");
        assert_eq!(format_duration(Duration::from_secs(3600)), "1h");
        assert_eq!(format_duration(Duration::from_secs(86400)), "1d");
    }

    fn rate_limited(secs: u32) -> RequestError {
        RequestError::RetryAfter(teloxide::types::Seconds::from_seconds(secs))
    }

    #[tokio::test]
    async fn rate_limited_call_is_retried_once() {
        let mut calls = 0;
        let result = retry_once(|| {
            calls += 1;
            let call = calls;
            async move {
                match call {
                    1 => Err(rate_limited(0)),
                    _ => Ok(call),
                }
            }
        })
        .await;

        assert_eq!(result.ok(), Some(2));
        assert_eq!(calls, 2);
    }

    #[tokio::test]
    async fn long_or_repeated_limits_give_up() {
        let mut calls = 0;
        let long = retry_once(|| {
            calls += 1;
            async { Err::<(), _>(rate_limited(3600)) }
        })
        .await;
        assert!(long.is_err());
        assert_eq!(calls, 1, "an hour-long wait is not waited out");

        let mut calls = 0;
        let repeated = retry_once(|| {
            calls += 1;
            async { Err::<(), _>(rate_limited(0)) }
        })
        .await;
        assert!(repeated.is_err());
        assert_eq!(calls, 2, "only one retry");
    }

    #[test]
    fn mute_end_is_never_permanent() {
        let unix_now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let now = DateTime::from_timestamp(unix_now.as_secs() as i64, 0).unwrap();
        let short = mute_until(Duration::from_secs(1)) - now;
        let huge = mute_until(Duration::from_secs(u32::MAX as u64)) - now;

        assert!(short.num_seconds() >= 30);
        assert!(huge.num_days() <= 366);
    }
}
