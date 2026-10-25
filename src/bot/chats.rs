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

/// After a failed fetch, the chat is not asked about again for this long.
const FAILURE_BACKOFF: Duration = Duration::from_secs(60);

/// Per-chat cache of [`ChatInfo`], refreshed after `admin_cache_ttl_secs`.
#[derive(Default)]
pub struct ChatCache {
    map: HashMap<i64, ChatInfo>,
    /// When fetching a chat's info last failed, cleared by a success.
    failed: HashMap<i64, Instant>,
}

impl ChatCache {
    /// Whether `chat` needs a fetch now: no fresh info and no recent failure.
    fn needs_fetch(&self, chat: i64, now: Instant, ttl: Duration) -> bool {
        let fresh = self
            .map
            .get(&chat)
            .is_some_and(|info| now.duration_since(info.fetched) < ttl);
        let backing_off = self
            .failed
            .get(&chat)
            .is_some_and(|at| now.duration_since(*at) < FAILURE_BACKOFF);
        !fresh && !backing_off
    }

    /// Store the outcome of a fetch made at `now`.
    fn record(&mut self, chat: i64, fetched: Option<ChatInfo>, now: Instant) {
        match fetched {
            Some(info) => {
                self.failed.remove(&chat);
                self.map.insert(chat, info);
            }
            None => {
                self.failed.insert(chat, now);
            }
        }
    }
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
///
/// The shared lock is never held across a Bot API call, so a slow chat does
/// not stall checks in every other chat. A failed fetch is not repeated for
/// [`FAILURE_BACKOFF`]; the old info (if any) is used meanwhile.
async fn with_chat_info<T>(
    bot: &Bot,
    state: &AppState,
    chat: ChatId,
    ttl: Duration,
    read: impl FnOnce(&ChatInfo) -> T,
) -> Option<T> {
    let now = Instant::now();
    let needs_fetch = state.chats.lock().await.needs_fetch(chat.0, now, ttl);
    let fetched = if needs_fetch {
        Some(fetch(bot, chat, now).await)
    } else {
        None
    };
    let mut cache = state.chats.lock().await;
    if let Some(fetched) = fetched {
        cache.record(chat.0, fetched, now);
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

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: Duration = Duration::from_secs(3600);

    fn info(fetched: Instant) -> ChatInfo {
        ChatInfo {
            fetched,
            admins: HashMap::from([(
                7,
                AdminRights {
                    delete_messages: true,
                    restrict_members: true,
                },
            )]),
            linked_channel: None,
        }
    }

    #[test]
    fn failures_are_not_retried_at_once() {
        let mut cache = ChatCache::default();
        let start = Instant::now();
        assert!(cache.needs_fetch(-1, start, TTL));

        cache.record(-1, None, start);

        assert!(!cache.needs_fetch(-1, start + Duration::from_secs(5), TTL));
        assert!(cache.needs_fetch(-1, start + FAILURE_BACKOFF, TTL));
        assert!(
            cache.needs_fetch(-2, start, TTL),
            "other chats are unaffected"
        );
    }

    #[test]
    fn success_is_cached_until_stale() {
        let mut cache = ChatCache::default();
        let start = Instant::now();
        cache.record(-1, None, start);

        cache.record(-1, Some(info(start)), start);

        assert!(cache.failed.is_empty());
        assert!(!cache.needs_fetch(-1, start + Duration::from_secs(60), TTL));
        assert!(cache.needs_fetch(-1, start + TTL, TTL));
    }
}
