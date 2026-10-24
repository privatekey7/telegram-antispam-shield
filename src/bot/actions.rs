//! Carrying out decisions: deleting, banning, muting and flood notices.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use teloxide::prelude::*;
use teloxide::types::{Chat, ChatId, ChatPermissions, MessageId, User};

use telegram_antispam_shield::config::Action;
use telegram_antispam_shield::flood::FloodReason;

use super::AppState;

/// Telegram's `deleteMessages` accepts at most this many IDs per call.
const DELETE_BATCH_LIMIT: usize = 100;
/// Telegram treats restrictions shorter than 30 s or longer than 366 days as
/// permanent, so mutes are kept inside that range.
const MIN_MUTE: Duration = Duration::from_secs(31);
const MAX_MUTE: Duration = Duration::from_secs(365 * 24 * 60 * 60);

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
}

/// Delete `ids` in as few calls as Telegram allows. Already-deleted messages
/// are fine: Telegram skips them.
pub async fn delete_all(bot: &Bot, chat: ChatId, ids: impl IntoIterator<Item = i32>) {
    let ids: Vec<MessageId> = ids.into_iter().map(MessageId).collect();
    for batch in ids.chunks(DELETE_BATCH_LIMIT) {
        if let Err(e) = bot.delete_messages(chat, batch.to_vec()).await {
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
            bot.ban_chat_member(chat, user.id).await.map(drop)
        }
        (Action::DeleteAndMute, Sender::User(user)) => bot
            .restrict_chat_member(chat, user.id, ChatPermissions::empty())
            .await
            .map(drop),
        (_, Sender::Channel(channel)) => bot.ban_chat_sender_chat(chat, channel.id).await.map(drop),
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, action = ?action, "punish_failed");
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
    let muted = bot
        .restrict_chat_member(chat, user.id, ChatPermissions::empty())
        .until_date(mute_until(flood.mute_for))
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
    let who = user.mention().unwrap_or_else(|| user.full_name());
    let length = format_duration(mute_for);
    // Plain text (no parse mode), so user names cannot inject markup.
    let notice = format!("🔇 {who} — мут {length} за флуд · muted {length} for flooding");

    let sent = match bot.send_message(chat, notice).await {
        Ok(sent) => sent,
        Err(e) => {
            tracing::warn!(error = %e, "flood_notice_failed");
            return;
        }
    };
    let bot = bot.clone();
    tokio::spawn(async move {
        tokio::time::sleep(ttl).await;
        if let Err(e) = bot.delete_message(chat, sent.id).await {
            tracing::debug!(error = %e, "flood_notice_delete_failed");
        }
    });
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
    fn durations_are_formatted_compactly() {
        assert_eq!(format_duration(Duration::from_secs(90)), "90s");
        assert_eq!(format_duration(Duration::from_secs(600)), "10m");
        assert_eq!(format_duration(Duration::from_secs(3600)), "1h");
        assert_eq!(format_duration(Duration::from_secs(86400)), "1d");
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
