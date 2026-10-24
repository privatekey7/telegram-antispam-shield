//! Telegram anti-spam bot entry point.
//!
//! Listens for group messages, stops flooders with [`flood::FloodGuard`],
//! classifies the rest with [`moderation::classify`], and deletes spam
//! (optionally banning/muting the author). Group admins and the group's own
//! posts (anonymous admins, the linked channel) are exempt; admin lists and
//! the linked channel are cached to limit API calls.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use teloxide::prelude::*;
use teloxide::types::{Chat, ChatId, ChatPermissions, MessageEntityKind, MessageId, User, UserId};
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

use telegram_antispam_shield::config::{self, Action, Settings};
use telegram_antispam_shield::flood::{FloodGuard, FloodReason, FloodVerdict, IncomingMessage};
use telegram_antispam_shield::model::Model;
use telegram_antispam_shield::moderation::{classify, MessageFacts, Verdict};

/// Telegram's `deleteMessages` accepts at most this many IDs per call.
const DELETE_BATCH_LIMIT: usize = 100;
/// Telegram treats restrictions shorter than 30 s or longer than 366 days as
/// permanent, so mutes are kept inside that range.
const MIN_MUTE: Duration = Duration::from_secs(31);
const MAX_MUTE: Duration = Duration::from_secs(365 * 24 * 60 * 60);
/// How often idle users are dropped from the flood tracker.
const FLOOD_SWEEP_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// What the bot remembers about a group between API calls.
struct ChatInfo {
    fetched: Instant,
    admins: HashSet<u64>,
    /// The channel whose posts are auto-forwarded into this group, if any.
    linked_channel: Option<i64>,
}

/// Per-chat cache of [`ChatInfo`], refreshed after `admin_cache_ttl_secs`.
#[derive(Default)]
struct ChatCache {
    map: HashMap<i64, ChatInfo>,
}

/// Who posted a message: a person, or a channel posting in the group.
enum Sender<'a> {
    User(&'a User),
    Channel(&'a Chat),
}

impl Sender<'_> {
    /// Telegram ID of the user or channel.
    fn id(&self) -> i64 {
        match self {
            Sender::User(user) => user.id.0 as i64,
            Sender::Channel(chat) => chat.id.0,
        }
    }
}

/// Shared application state injected into every handler.
struct AppState {
    model: Model,
    settings: Settings,
    chats: Mutex<ChatCache>,
    /// Held only for synchronous checks, never across an `.await`.
    flood: std::sync::Mutex<FloodGuard>,
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    init_logging();

    let settings = Settings::from_env();
    let model = Model::load_default()?;
    tracing::info!(
        buckets = model.num_buckets,
        threshold = settings.threshold.unwrap_or(model.threshold),
        action = ?settings.action,
        dry_run = settings.dry_run,
        flood_enabled = settings.flood.enabled,
        "model_loaded"
    );

    let bot = Bot::new(config::bot_token()?);
    let state = Arc::new(AppState {
        model,
        flood: std::sync::Mutex::new(FloodGuard::new(settings.flood.clone())),
        settings,
        chats: Mutex::new(ChatCache::default()),
    });
    spawn_flood_sweeper(Arc::clone(&state));

    let handler = Update::filter_message().endpoint(on_message);
    let mut dispatcher = Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state])
        .enable_ctrlc_handler()
        .build();

    // Container runtimes stop the bot with SIGTERM — shut down gracefully.
    // Unix-only; elsewhere (local dev on Windows) Ctrl+C is handled above.
    #[cfg(unix)]
    let shutdown = dispatcher.shutdown_token();
    #[cfg(unix)]
    tokio::spawn(async move {
        if let Ok(mut sigterm) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sigterm.recv().await;
            tracing::info!("sigterm_received");
            if let Ok(fut) = shutdown.shutdown() {
                fut.await;
            }
        }
    });

    tracing::info!("bot_started");
    dispatcher.dispatch().await;
    tracing::info!("bot_stopped");
    Ok(())
}

async fn on_message(bot: Bot, msg: Message, state: Arc<AppState>) -> ResponseResult<()> {
    // Private chat: answer /start, /help and /status so admins can verify it.
    if msg.chat.is_private() {
        if let Some(t) = msg.text() {
            if t.starts_with("/start") || t.starts_with("/help") {
                bot.send_message(msg.chat.id, START_TEXT).await?;
            } else if t.starts_with("/status") {
                // Only the configured owner may see the bot's configuration.
                let from_owner = msg
                    .from
                    .as_ref()
                    .map(|u| state.settings.is_owner(u.id.0 as i64))
                    .unwrap_or(false);
                if from_owner {
                    bot.send_message(msg.chat.id, status_text(&state)).await?;
                }
            }
        }
        return Ok(());
    }

    // Only moderate groups and supergroups.
    if !(msg.chat.is_group() || msg.chat.is_supergroup()) {
        return Ok(());
    }

    // Enforce the chat allowlist before any work (including reading text), so
    // the bot ignores — and optionally leaves — chats it was not authorised for.
    if !state.settings.chat_allowed(msg.chat.id.0) {
        if state.settings.leave_unknown_chats {
            match bot.leave_chat(msg.chat.id).await {
                Ok(_) => tracing::info!(chat = msg.chat.id.0, "left_unauthorized_chat"),
                Err(e) => tracing::warn!(error = %e, chat = msg.chat.id.0, "leave_failed"),
            }
        } else {
            tracing::debug!(chat = msg.chat.id.0, "ignored_unauthorized_chat");
        }
        return Ok(());
    }

    // The group's own posts: anonymous admins, auto-forwards from the linked
    // channel and comments made as that channel.
    if msg.is_automatic_forward() || is_own_post(&bot, &state, &msg).await {
        return Ok(());
    }
    let Some(sender) = sender_of(&msg) else {
        return Ok(()); // a bot
    };
    if let Sender::User(user) = sender {
        if state.settings.exempt_admins && is_admin(&bot, &state, msg.chat.id, user.id).await {
            return Ok(());
        }
    }

    let text = msg.text().or_else(|| msg.caption());

    // Flood rules see every message, including stickers and media without text.
    if let FloodVerdict::Flood {
        reason,
        message_ids,
        mute_for,
        strike,
    } = check_flood(&state, &msg, sender.id(), text)
    {
        let flood = FloodAction {
            reason,
            message_ids,
            mute_for,
            strike,
        };
        return punish_flood(&bot, &state, msg.chat.id, &sender, flood).await;
    }

    let visible = text.unwrap_or_default();
    let links = hidden_links(&msg, visible);
    let facts = MessageFacts {
        text: visible,
        hidden_links: &links,
        inline_keyboard: has_inline_keyboard(&msg),
    };
    let Verdict::Spam { score, reason } = classify(&state.model, &state.settings, &facts) else {
        return Ok(());
    };

    tracing::info!(
        chat = msg.chat.id.0,
        user = sender.id(),
        score = score,
        reason = reason.as_str(),
        dry_run = state.settings.dry_run,
        "spam_detected"
    );

    if state.settings.dry_run {
        return Ok(());
    }

    if let Err(e) = bot.delete_message(msg.chat.id, msg.id).await {
        tracing::warn!(error = %e, "delete_failed (is the bot an admin with delete rights?)");
        return Ok(());
    }
    punish_spammer(&bot, state.settings.action, msg.chat.id, &sender).await;
    Ok(())
}

/// The author of a message, or `None` for bots. Anonymous admins and the
/// linked channel are filtered out earlier by [`is_own_post`].
fn sender_of(msg: &Message) -> Option<Sender<'_>> {
    if let Some(chat) = &msg.sender_chat {
        return Some(Sender::Channel(chat));
    }
    let user = msg.from.as_ref()?;
    (!user.is_bot).then_some(Sender::User(user))
}

/// Whether the group itself posted `msg`: as an anonymous admin (the sender
/// is the group) or as its linked channel.
async fn is_own_post(bot: &Bot, state: &AppState, msg: &Message) -> bool {
    let Some(sender_chat) = &msg.sender_chat else {
        return false;
    };
    if sender_chat.id == msg.chat.id {
        return true;
    }
    with_chat_info(bot, state, msg.chat.id, |info| {
        info.linked_channel == Some(sender_chat.id.0)
    })
    .await
}

/// Targets of links hidden under words; links already visible are skipped.
fn hidden_links(msg: &Message, visible: &str) -> Vec<String> {
    msg.entities()
        .or_else(|| msg.caption_entities())
        .unwrap_or_default()
        .iter()
        .filter_map(|entity| match &entity.kind {
            MessageEntityKind::TextLink { url } => Some(url.as_str()),
            _ => None,
        })
        .filter(|url| !visible.contains(url))
        .map(str::to_owned)
        .collect()
}

/// Inline buttons on a message that did not come through an inline bot.
fn has_inline_keyboard(msg: &Message) -> bool {
    msg.via_bot.is_none()
        && msg
            .reply_markup()
            .is_some_and(|markup| markup.inline_keyboard.iter().any(|row| !row.is_empty()))
}

/// Apply the configured action to a spammer. Channels cannot be muted, so
/// they are banned whenever the action goes beyond deleting.
async fn punish_spammer(bot: &Bot, action: Action, chat: ChatId, sender: &Sender<'_>) {
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

/// What to do with a flooder, as decided by [`FloodGuard::check`].
struct FloodAction {
    reason: FloodReason,
    message_ids: Vec<i32>,
    mute_for: Duration,
    strike: u32,
}

fn check_flood(
    state: &AppState,
    msg: &Message,
    sender_id: i64,
    text: Option<&str>,
) -> FloodVerdict {
    let incoming = IncomingMessage {
        chat_id: msg.chat.id.0,
        sender_id,
        message_id: msg.id.0,
        text,
        media_group_id: msg.media_group_id().map(|g| g.0.as_str()),
    };
    // A poisoned lock only means another handler panicked; the data is still usable.
    let mut guard = state.flood.lock().unwrap_or_else(|e| e.into_inner());
    guard.check(incoming, Instant::now())
}

/// Delete the flood messages, mute the author and optionally tell the chat.
/// Channels cannot be muted, so their flood is only deleted.
async fn punish_flood(
    bot: &Bot,
    state: &AppState,
    chat: ChatId,
    sender: &Sender<'_>,
    flood: FloodAction,
) -> ResponseResult<()> {
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
        return Ok(());
    }

    let ids: Vec<MessageId> = flood.message_ids.into_iter().map(MessageId).collect();
    for batch in ids.chunks(DELETE_BATCH_LIMIT) {
        if let Err(e) = bot.delete_messages(chat, batch.to_vec()).await {
            tracing::warn!(error = %e, "flood_delete_failed (is the bot an admin with delete rights?)");
        }
    }

    let Sender::User(user) = sender else {
        return Ok(());
    };
    let muted = bot
        .restrict_chat_member(chat, user.id, ChatPermissions::empty())
        .until_date(mute_until(flood.mute_for))
        .await;
    if let Err(e) = muted {
        tracing::warn!(error = %e, "flood_mute_failed (grant the bot the \"Ban users\" right)");
        return Ok(());
    }

    if state.settings.flood.notify {
        notify_flood_mute(
            bot,
            chat,
            user,
            flood.mute_for,
            state.settings.flood.notify_ttl,
        )
        .await;
    }
    Ok(())
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

/// Periodically drop idle users from the flood tracker to bound memory.
fn spawn_flood_sweeper(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(FLOOD_SWEEP_INTERVAL);
        loop {
            interval.tick().await;
            let mut guard = state.flood.lock().unwrap_or_else(|e| e.into_inner());
            guard.sweep(Instant::now());
            tracing::debug!(tracked = guard.tracked_users(), "flood_sweep");
        }
    });
}

/// Returns whether `user` is an administrator of `chat`, using a TTL cache.
async fn is_admin(bot: &Bot, state: &AppState, chat: ChatId, user: UserId) -> bool {
    with_chat_info(bot, state, chat, |info| info.admins.contains(&user.0)).await
}

/// Run `read` on the cached [`ChatInfo`] for `chat`, refreshing it first when
/// stale. Returns `false` when the info cannot be fetched at all.
async fn with_chat_info(
    bot: &Bot,
    state: &AppState,
    chat: ChatId,
    read: impl FnOnce(&ChatInfo) -> bool,
) -> bool {
    let ttl = Duration::from_secs(state.settings.admin_cache_ttl_secs);
    let now = Instant::now();
    let mut cache = state.chats.lock().await;

    let fresh = cache
        .map
        .get(&chat.0)
        .is_some_and(|info| now.duration_since(info.fetched) < ttl);
    if !fresh {
        if let Some(info) = fetch_chat_info(bot, chat, now).await {
            cache.map.insert(chat.0, info);
        }
    }
    cache.map.get(&chat.0).is_some_and(read)
}

/// Admins and linked channel of `chat`; `None` if the admin list is unavailable.
async fn fetch_chat_info(bot: &Bot, chat: ChatId, now: Instant) -> Option<ChatInfo> {
    let admins = match bot.get_chat_administrators(chat).await {
        Ok(admins) => admins.iter().map(|m| m.user.id.0).collect(),
        Err(e) => {
            tracing::warn!(error = %e, chat = chat.0, "admin_list_failed");
            return None;
        }
    };
    let linked_channel = match bot.get_chat(chat).await {
        Ok(full) => full.linked_chat_id(),
        Err(e) => {
            tracing::warn!(error = %e, chat = chat.0, "chat_info_failed");
            None
        }
    };
    Some(ChatInfo {
        fetched: now,
        admins,
        linked_channel,
    })
}

const START_TEXT: &str = "🛡 Anti-Spam Shield is running.\n\n\
Add me to your group and grant the \"Delete messages\" admin right. \
I will automatically remove spam in Russian and English.\n\n\
Commands: /status";

fn status_text(state: &AppState) -> String {
    let allowed = match &state.settings.allowed_chats {
        None => "all chats".to_string(),
        Some(set) => format!("{} chat(s)", set.len()),
    };
    let flood = &state.settings.flood;
    let flood_line = if flood.enabled {
        format!(
            ">{} msgs/{}s, {} repeats/{}s",
            flood.max_messages,
            flood.rate_window.as_secs(),
            flood.max_duplicates,
            flood.duplicate_window.as_secs()
        )
    } else {
        "off".to_string()
    };
    format!(
        "🛡 Anti-Spam Shield\nthreshold: {:.2}\naction: {:?}\ndry_run: {}\njob_offers: {}\nexempt_admins: {}\nallowed_chats: {}\nleave_unknown_chats: {}\nflood: {}",
        state.settings.threshold.unwrap_or(state.model.threshold),
        state.settings.action,
        state.settings.dry_run,
        if state.settings.block_job_offers {
            "blocked"
        } else {
            "allowed"
        },
        state.settings.exempt_admins,
        allowed,
        state.settings.leave_unknown_chats,
        flood_line,
    )
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
