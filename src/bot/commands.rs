//! Admin commands in a group, sent as a reply to the offending message:
//!
//! * `/spam` — delete it and the author's recent messages, ban the author and
//!   learn the text, so similar messages are deleted automatically;
//! * `/ban`  — the same without learning;
//! * `/warn` — delete it and warn the author; `ANTISPAM_WARN_LIMIT` warnings
//!   within a month mean a ban.
//!
//! These are explicit admin actions, so they work in dry-run mode too. They
//! need the "Ban users" right (`/warn` only "Delete messages" when warnings
//! never ban); anonymous admins are refused (see `access`).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use teloxide::prelude::*;

use telegram_antispam_shield::access::{Access, AdminAction};
use telegram_antispam_shield::config::Action;

use super::actions::{self, Sender};
use super::{rights, AppState};

/// How long a warning notice stays in the chat.
const WARNING_NOTICE_TTL: Duration = Duration::from_secs(60);
/// How long a hint about a misused command stays in the chat.
const HINT_TTL: Duration = Duration::from_secs(20);

const NO_TARGET_HINT: &str = "↩️ Ответьте этой командой на сообщение нарушителя. \
Сообщения, отправленные до того, как группа стала супергруппой, боту недоступны: \
удалите их вручную. · Reply to the offending message with this command. Messages \
from before the group became a supergroup are out of reach: delete them by hand.";
const BOT_TARGET_HINT: &str = "🤖 Команды не действуют на ботов. · Commands do not apply to bots.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Spam,
    Ban,
    Warn,
}

impl Command {
    /// The admin action, for the rights check.
    pub fn action(self) -> AdminAction {
        match self {
            Command::Spam => AdminAction::Spam,
            Command::Ban => AdminAction::Ban,
            Command::Warn => AdminAction::Warn,
        }
    }
}

/// The command in `text`, if it is one of ours. `/spam@OtherBot` is not.
pub fn parse(text: &str, bot_username: &str) -> Option<Command> {
    let first = text.split_whitespace().next()?.strip_prefix('/')?;
    let (name, target) = match first.split_once('@') {
        Some((name, target)) => (name, Some(target)),
        None => (first, None),
    };
    if target.is_some_and(|t| !t.eq_ignore_ascii_case(bot_username)) {
        return None;
    }
    match name.to_ascii_lowercase().as_str() {
        "spam" => Some(Command::Spam),
        "ban" => Some(Command::Ban),
        "warn" => Some(Command::Warn),
        _ => None,
    }
}

/// Carry out `command` sent as `msg` by someone the rights check allowed.
pub async fn run(bot: &Bot, state: &AppState, msg: &Message, command: Command) {
    let chat = msg.chat.id;
    // The command itself is noise once handled.
    actions::delete_all(bot, chat, [msg.id.0]).await;

    // Without a hint a misused command just vanishes and the admin is left
    // guessing. A reply to a message from another chat (including the group
    // before it became a supergroup) also arrives without a target.
    let Some(target) = msg.reply_to_message() else {
        actions::send_temporary(bot, chat, NO_TARGET_HINT.to_owned(), HINT_TTL).await;
        return;
    };
    let Some(sender) = actions::sender_of(target) else {
        actions::send_temporary(bot, chat, BOT_TARGET_HINT.to_owned(), HINT_TTL).await;
        return;
    };
    let text = target
        .text()
        .or_else(|| target.caption())
        .unwrap_or_default();
    tracing::info!(
        chat = chat.0,
        user = sender.id(),
        admin = msg.from.as_ref().map_or(0, |u| u.id.0),
        command = ?command,
        "admin_command"
    );

    match command {
        Command::Spam | Command::Ban => {
            let recent = {
                let mut shield = state.shield();
                if command == Command::Spam {
                    shield.learn_spam(chat.0, sender.id(), text);
                }
                shield.recent_from(chat.0, sender.id())
            };
            actions::delete_all(bot, chat, recent.into_iter().chain([target.id.0])).await;
            actions::punish_spammer(bot, Action::DeleteAndBan, chat, &sender).await;
        }
        Command::Warn => {
            actions::delete_all(bot, chat, [target.id.0]).await;
            let (count, ban) = state.shield().warn(chat.0, sender.id(), unix_now());
            if ban {
                actions::punish_spammer(bot, Action::DeleteAndBan, chat, &sender).await;
            }
            warn_notice(bot, state, chat, &sender, count, ban).await;
        }
    }
}

/// Turn down a command from an admin without the right it needs, or from an
/// anonymous admin: delete it and say why for a few seconds.
pub async fn refuse(bot: &Bot, msg: &Message, access: Access) {
    tracing::info!(chat = msg.chat.id.0, access = ?access, "admin_command_refused");
    actions::delete_all(bot, msg.chat.id, [msg.id.0]).await;
    let text = format!("🔒 {}", rights::refusal_text(access));
    actions::send_temporary(bot, msg.chat.id, text, HINT_TTL).await;
}

/// Tell the chat about a warning; the notice deletes itself after a minute.
async fn warn_notice(
    bot: &Bot,
    state: &AppState,
    chat: ChatId,
    sender: &Sender<'_>,
    count: u32,
    banned: bool,
) {
    let who = sender.notice_name();
    let limit = state.settings.warn_limit;
    // Plain text (no parse mode), so user names cannot inject markup.
    let notice = if banned {
        format!("⛔ {who} — бан после {count} предупреждений · banned after {count} warnings")
    } else if limit > 0 {
        format!("⚠️ {who} — предупреждение {count}/{limit} · warning {count}/{limit}")
    } else {
        format!("⚠️ {who} — предупреждение · warning")
    };
    actions::send_temporary(bot, chat, notice, WARNING_NOTICE_TTL).await;
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_our_commands_only() {
        assert_eq!(parse("/spam", "ShieldBot"), Some(Command::Spam));
        assert_eq!(
            parse("/SPAM@shieldbot please", "ShieldBot"),
            Some(Command::Spam)
        );
        assert_eq!(parse("/ban", "ShieldBot"), Some(Command::Ban));
        assert_eq!(parse("/warn за флуд", "ShieldBot"), Some(Command::Warn));
        assert_eq!(parse("/spam@OtherBot", "ShieldBot"), None);
        assert_eq!(parse("/start", "ShieldBot"), None);
        assert_eq!(parse("это не /spam", "ShieldBot"), None);
    }
}
