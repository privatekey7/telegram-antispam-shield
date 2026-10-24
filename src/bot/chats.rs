//! Per-group facts from the Bot API (admins, linked channel), cached.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use teloxide::prelude::*;
use teloxide::types::{ChatId, UserId};

use super::AppState;

/// What the bot remembers about a group between API calls.
pub struct ChatInfo {
    fetched: Instant,
    admins: HashSet<u64>,
    /// The channel whose posts are auto-forwarded into this group, if any.
    linked_channel: Option<i64>,
}

/// Per-chat cache of [`ChatInfo`], refreshed after `admin_cache_ttl_secs`.
#[derive(Default)]
pub struct ChatCache {
    map: HashMap<i64, ChatInfo>,
}

/// Whether `user` is an administrator of `chat`.
pub async fn is_admin(bot: &Bot, state: &AppState, chat: ChatId, user: UserId) -> bool {
    with_chat_info(bot, state, chat, |info| info.admins.contains(&user.0)).await
}

/// Whether `channel` is the channel linked to `chat`.
pub async fn is_linked_channel(bot: &Bot, state: &AppState, chat: ChatId, channel: ChatId) -> bool {
    with_chat_info(bot, state, chat, |info| {
        info.linked_channel == Some(channel.0)
    })
    .await
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
        if let Some(info) = fetch(bot, chat, now).await {
            cache.map.insert(chat.0, info);
        }
    }
    cache.map.get(&chat.0).is_some_and(read)
}

/// Admins and linked channel of `chat`; `None` if the admin list is unavailable.
async fn fetch(bot: &Bot, chat: ChatId, now: Instant) -> Option<ChatInfo> {
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
