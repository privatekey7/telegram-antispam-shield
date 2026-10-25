//! Checking an admin request against the rights it needs (see
//! [`telegram_antispam_shield::access`] for the policy).

use teloxide::prelude::*;
use teloxide::types::{ChatId, User};

use telegram_antispam_shield::access::{self, Access, AdminAction, Caller, Privilege};

use super::{chats, AppState};

/// Whether `user` (or an anonymous admin, when `as_group`) may ask for
/// `action` in `chat`.
///
/// The cached admin list rejects ordinary members without an API call; an
/// admin the cache allows is confirmed with `getChatMember`, so a demoted
/// admin loses access at once instead of after the cache TTL.
pub async fn authorize(
    bot: &Bot,
    state: &AppState,
    chat: ChatId,
    user: Option<&User>,
    as_group: bool,
    action: AdminAction,
) -> Access {
    let needed = action.privileges(state.settings.warn_limit);
    let caller = match user {
        _ if as_group => Caller::AnonymousAdmin,
        None => Caller::Member(None),
        Some(user) if state.settings.is_owner(user.id.0 as i64) => Caller::Owner,
        Some(user) => Caller::Member(chats::cached_rights(bot, state, chat, user.id).await),
    };
    let access = access::check(caller, needed);
    match (caller, access, user) {
        (Caller::Member(_), Access::Allowed, Some(user)) => {
            let live = chats::live_rights(bot, chat, user.id).await;
            access::check(Caller::Member(live), needed)
        }
        _ => access,
    }
}

/// A short bilingual explanation of a refusal, for the chat or a button.
pub fn refusal_text(access: Access) -> &'static str {
    match access {
        Access::Allowed => "",
        Access::NotAdmin => "Только для админов · Admins only",
        Access::MissingRight(Privilege::RestrictMembers) => {
            "Нужно право «Блокировка пользователей» · Needs the \"Ban users\" admin right"
        }
        Access::MissingRight(Privilege::DeleteMessages) => {
            "Нужно право «Удаление сообщений» · Needs the \"Delete messages\" admin right"
        }
        Access::Anonymous => {
            "Анонимным админам команды недоступны: выключите «Анонимность» или \
             попросите админа с правом «Блокировка пользователей». · Commands do not \
             work for anonymous admins: turn off \"Remain anonymous\" or ask an admin \
             with the \"Ban users\" right."
        }
    }
}
