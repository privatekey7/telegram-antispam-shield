//! Per-group facts from the Bot API (admins, linked channel), cached.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use teloxide::prelude::*;
use teloxide::types::{ChatId, ChatMemberKind, UserId};

use telegram_antispam_shield::access::AdminRights;

use super::AppState;

/// A privileged request from someone not in the cached admin list refreshes
/// the list at most this often, so a new admin waits a minute at most and
/// strangers typing `/ban` cannot make the bot hammer the API.
const MISS_REFRESH: Duration = Duration::from_secs(60);

/// What the bot remembers about a group between API calls.
pub struct ChatInfo {
    fetched: Instant,
    /// Admin user IDs and their rights.
    admins: HashMap<u64, AdminRights>,
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
    let ttl = cache_ttl(state);
    with_chat_info(bot, state, chat, ttl, |info| {
        info.admins.contains_key(&user.0)
    })
    .await
    .unwrap_or(false)
}

/// The cached rights of `user` in `chat`, or `None` if not an admin. A miss
/// refreshes the list if it is older than [`MISS_REFRESH`].
pub async fn cached_rights(
    bot: &Bot,
    state: &AppState,
    chat: ChatId,
    user: UserId,
) -> Option<AdminRights> {
    let lookup = |info: &ChatInfo| info.admins.get(&user.0).copied();
    let ttl = cache_ttl(state);
    if let Some(rights) = with_chat_info(bot, state, chat, ttl, lookup)
        .await
        .flatten()
    {
        return Some(rights);
    }
    with_chat_info(bot, state, chat, ttl.min(MISS_REFRESH), lookup)
        .await
        .flatten()
}

/// The rights of `user` in `chat` right now, asked from Telegram, so an admin
/// demoted within the cache TTL loses access at once. `None` if not an admin
/// or if Telegram cannot be asked (fail closed).
pub async fn live_rights(bot: &Bot, chat: ChatId, user: UserId) -> Option<AdminRights> {
    match bot.get_chat_member(chat, user).await {
        Ok(member) => rights_of(&member.kind),
        Err(e) => {
            tracing::warn!(error = %e, chat = chat.0, user = user.0, "member_check_failed");
            None
        }
    }
}

/// The rights the bot cares about; `None` for anyone but admins.
fn rights_of(kind: &ChatMemberKind) -> Option<AdminRights> {
    kind.is_privileged().then(|| AdminRights {
        delete_messages: kind.can_delete_messages(),
        restrict_members: kind.can_restrict_members(),
    })
}

fn cache_ttl(state: &AppState) -> Duration {
    Duration::from_secs(state.settings.admin_cache_ttl_secs)
}

/// Whether `channel` is the channel linked to `chat`.
pub async fn is_linked_channel(bot: &Bot, state: &AppState, chat: ChatId, channel: ChatId) -> bool {
    let ttl = cache_ttl(state);
    with_chat_info(bot, state, chat, ttl, |info| {
        info.linked_channel == Some(channel.0)
    })
    .await
    .unwrap_or(false)
}

/// Run `read` on the cached [`ChatInfo`] for `chat`, refreshing it first when
/// older than `ttl`. Returns `None` when the info cannot be fetched at all.
async fn with_chat_info<T>(
    bot: &Bot,
    state: &AppState,
    chat: ChatId,
    ttl: Duration,
    read: impl FnOnce(&ChatInfo) -> T,
) -> Option<T> {
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
    cache.map.get(&chat.0).map(read)
}

/// Admins and linked channel of `chat`; `None` if the admin list is unavailable.
async fn fetch(bot: &Bot, chat: ChatId, now: Instant) -> Option<ChatInfo> {
    let admins = match bot.get_chat_administrators(chat).await {
        Ok(admins) => admins
            .iter()
            .filter_map(|m| Some((m.user.id.0, rights_of(&m.kind)?)))
            .collect(),
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
