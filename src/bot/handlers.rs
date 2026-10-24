//! Update handlers: group messages, member joins and private commands.

use std::sync::Arc;
use std::time::Instant;

use teloxide::prelude::*;
use teloxide::types::{ChatMemberUpdated, MessageEntityKind, MessageKind, User};

use telegram_antispam_shield::access::Access;
use telegram_antispam_shield::config::Action;
use telegram_antispam_shield::flood::{FloodVerdict, IncomingMessage};
use telegram_antispam_shield::moderation::{MessageFacts, Verdict};
use telegram_antispam_shield::shield::Incoming;

use super::actions::{self, FloodAction, Sender};
use super::reports::{self, Incident};
use super::{chats, commands, rights, AppState};

const START_TEXT: &str = "🛡 Anti-Spam Shield is running.\n\n\
Add me to your group and grant the \"Delete messages\" admin right. \
I will automatically remove spam in Russian and English.\n\n\
Private: /status (owner). In a group, admins with the \"Ban users\" right \
reply to a message with /spam (delete, ban, learn), /ban or /warn.";

/// What `/status` tells anyone but the owner: nothing about the configuration.
const STATUS_FOR_OTHERS: &str = "🛡 Anti-Spam Shield is running. \
/status details are only for the bot's owner (ANTISPAM_OWNER_ID).";

pub async fn on_message(bot: Bot, msg: Message, state: Arc<AppState>) -> ResponseResult<()> {
    if msg.chat.is_private() {
        return on_private_message(&bot, &msg, &state).await;
    }
    // Only moderate groups and supergroups.
    if !(msg.chat.is_group() || msg.chat.is_supergroup()) {
        return Ok(());
    }
    if !chat_allowed(&bot, &state, msg.chat.id).await {
        return Ok(());
    }

    if let Some(members) = msg.new_chat_members() {
        for member in members.iter().filter(|m| !m.is_bot) {
            on_join(&bot, &state, msg.chat.id, member).await;
        }
        delete_service_message(&bot, &state, &msg).await;
        return Ok(());
    }
    if msg.left_chat_member().is_some() {
        delete_service_message(&bot, &state, &msg).await;
        return Ok(());
    }

    if let Some(command) = msg
        .text()
        .and_then(|t| commands::parse(t, &state.bot_username))
    {
        let as_group = msg
            .sender_chat
            .as_ref()
            .is_some_and(|c| c.id == msg.chat.id);
        let action = command.action();
        let from = msg.from.as_ref();
        match rights::authorize(&bot, &state, msg.chat.id, from, as_group, action).await {
            Access::Allowed => {
                commands::run(&bot, &state, &msg, command).await;
                return Ok(());
            }
            // Not an admin: an ordinary message, moderated below.
            Access::NotAdmin => {}
            refused => {
                commands::refuse(&bot, &msg, refused).await;
                return Ok(());
            }
        }
    }

    // The group's own posts: anonymous admins, auto-forwards from the linked
    // channel and comments made as that channel.
    if msg.is_automatic_forward() || is_own_post(&bot, &state, &msg).await {
        return Ok(());
    }
    let Some(sender) = actions::sender_of(&msg) else {
        return Ok(()); // a bot
    };
    if let Sender::User(user) = sender {
        if state.settings.exempt_admins && chats::is_admin(&bot, &state, msg.chat.id, user.id).await
        {
            return Ok(());
        }
    }

    let text = msg.text().or_else(|| msg.caption());

    // Flood rules see every message, including stickers and media without text.
    if let Some(flood) = check_flood(&state, &msg, sender.id(), text) {
        actions::punish_flood(&bot, &state, msg.chat.id, &sender, flood).await;
        return Ok(());
    }

    // Only untrusted members are looked up in the CAS database.
    let trusted = state.shield().is_trusted(msg.chat.id.0, sender.id());
    let known_spammer = match sender {
        Sender::User(user) if !trusted => state.cas.is_listed(user.id).await,
        _ => false,
    };
    let visible = text.unwrap_or_default();
    let links = hidden_links(&msg, visible);
    let incoming = Incoming {
        chat: msg.chat.id.0,
        sender: sender.id(),
        message_id: msg.id.0,
        facts: MessageFacts {
            text: visible,
            hidden_links: &links,
            inline_keyboard: has_inline_keyboard(&msg),
            has_link: has_link_entity(&msg),
            forwarded: is_forwarded(&msg),
        },
        known_spammer,
    };
    let (decision, recent) = {
        let mut shield = state.shield();
        let decision = shield.inspect(&incoming, Instant::now());
        let recent = shield.recent_from(incoming.chat, incoming.sender);
        (decision, recent)
    };

    let Verdict::Spam { score, reason } = decision.verdict else {
        if decision.newly_trusted {
            tracing::debug!(
                chat = incoming.chat,
                user = incoming.sender,
                "member_trusted"
            );
        }
        return Ok(());
    };
    tracing::info!(
        chat = incoming.chat,
        user = incoming.sender,
        score = score,
        reason = reason.as_str(),
        also_deleted = decision.also_delete.len(),
        dry_run = state.settings.dry_run,
        "spam_detected"
    );
    if state.settings.dry_run {
        return Ok(());
    }

    let mut doomed = vec![msg.id.0];
    doomed.extend(decision.also_delete);
    let punishes = state.settings.action != Action::DeleteOnly;
    if punishes && state.settings.cleanup_on_ban {
        // The spammer's other recent messages go too.
        doomed.extend(recent);
    }
    doomed.sort_unstable();
    doomed.dedup();
    let deleted = doomed.len();
    actions::delete_all(&bot, msg.chat.id, doomed).await;
    actions::punish_spammer(&bot, state.settings.action, msg.chat.id, &sender).await;
    let incident = Incident {
        chat: &msg.chat,
        sender: &sender,
        text: visible,
        reason,
        score,
        deleted,
    };
    reports::send(&bot, &state, incident).await;
    Ok(())
}

/// Joins seen as `chat_member` updates (large groups may hide join messages).
pub async fn on_chat_member(
    bot: Bot,
    update: ChatMemberUpdated,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    let joined =
        !update.old_chat_member.kind.is_present() && update.new_chat_member.kind.is_present();
    let user = &update.new_chat_member.user;
    if joined && !user.is_bot && chat_allowed(&bot, &state, update.chat.id).await {
        on_join(&bot, &state, update.chat.id, user).await;
    }
    Ok(())
}

/// Remember the join for the newcomer rule and stop known spammers at the
/// door when the action allows banning or muting.
async fn on_join(bot: &Bot, state: &AppState, chat: ChatId, user: &User) {
    state
        .shield()
        .joined(chat.0, user.id.0 as i64, Instant::now());
    if state.settings.action == Action::DeleteOnly || !state.cas.is_listed(user.id).await {
        return;
    }
    tracing::info!(
        chat = chat.0,
        user = user.id.0,
        dry_run = state.settings.dry_run,
        "cas_listed_member_joined"
    );
    if !state.settings.dry_run {
        actions::punish_spammer(bot, state.settings.action, chat, &Sender::User(user)).await;
    }
}

/// Delete a "joined" / "left" notice when configured.
async fn delete_service_message(bot: &Bot, state: &AppState, msg: &Message) {
    if state.settings.delete_service_messages && !state.settings.dry_run {
        actions::delete_all(bot, msg.chat.id, [msg.id.0]).await;
    }
}

/// Private chat: answer /start, /help and /status so admins can verify it.
async fn on_private_message(bot: &Bot, msg: &Message, state: &AppState) -> ResponseResult<()> {
    let Some(text) = msg.text() else {
        return Ok(());
    };
    if text.starts_with("/start") || text.starts_with("/help") {
        bot.send_message(msg.chat.id, START_TEXT).await?;
    } else if text.starts_with("/status") {
        // Only the configured owner may see the bot's configuration: the
        // threshold and limits would help a spammer tune around them.
        let from_owner = msg
            .from
            .as_ref()
            .is_some_and(|u| state.settings.is_owner(u.id.0 as i64));
        let text = if from_owner {
            status_text(state)
        } else {
            STATUS_FOR_OTHERS.to_owned()
        };
        bot.send_message(msg.chat.id, text).await?;
    }
    Ok(())
}

/// Enforce the chat allowlist before any work (including reading text), so
/// the bot ignores — and optionally leaves — chats it was not authorised for.
async fn chat_allowed(bot: &Bot, state: &AppState, chat: ChatId) -> bool {
    if state.settings.chat_allowed(chat.0) {
        return true;
    }
    if state.settings.leave_unknown_chats {
        match bot.leave_chat(chat).await {
            Ok(_) => tracing::info!(chat = chat.0, "left_unauthorized_chat"),
            Err(e) => tracing::warn!(error = %e, chat = chat.0, "leave_failed"),
        }
    } else {
        tracing::debug!(chat = chat.0, "ignored_unauthorized_chat");
    }
    false
}

/// Whether the group itself posted `msg`: as an anonymous admin (the sender
/// is the group) or as its linked channel.
async fn is_own_post(bot: &Bot, state: &AppState, msg: &Message) -> bool {
    match &msg.sender_chat {
        None => false,
        Some(sender) if sender.id == msg.chat.id => true,
        Some(sender) => chats::is_linked_channel(bot, state, msg.chat.id, sender.id).await,
    }
}

fn check_flood(
    state: &AppState,
    msg: &Message,
    sender_id: i64,
    text: Option<&str>,
) -> Option<FloodAction> {
    let incoming = IncomingMessage {
        chat_id: msg.chat.id.0,
        sender_id,
        message_id: msg.id.0,
        text,
        media_group_id: msg.media_group_id().map(|g| g.0.as_str()),
    };
    match state.flood().check(incoming, Instant::now()) {
        FloodVerdict::Allow => None,
        FloodVerdict::Flood {
            reason,
            message_ids,
            mute_for,
            strike,
        } => Some(FloodAction {
            reason,
            message_ids,
            mute_for,
            strike,
        }),
    }
}

fn entities(msg: &Message) -> &[teloxide::types::MessageEntity] {
    msg.entities()
        .or_else(|| msg.caption_entities())
        .unwrap_or_default()
}

/// Targets of links hidden under words; links already visible are skipped.
fn hidden_links(msg: &Message, visible: &str) -> Vec<String> {
    entities(msg)
        .iter()
        .filter_map(|entity| match &entity.kind {
            MessageEntityKind::TextLink { url } => Some(url.as_str()),
            _ => None,
        })
        .filter(|url| !visible.contains(url))
        .map(str::to_owned)
        .collect()
}

/// Whether Telegram marked a link in the text.
fn has_link_entity(msg: &Message) -> bool {
    entities(msg).iter().any(|entity| {
        matches!(
            entity.kind,
            MessageEntityKind::Url | MessageEntityKind::TextLink { .. }
        )
    })
}

/// Forwarded, or replying with a quote from another chat, or a story.
fn is_forwarded(msg: &Message) -> bool {
    let external_reply = match &msg.kind {
        MessageKind::Common(common) => common.external_reply.is_some(),
        _ => false,
    };
    msg.forward_origin().is_some() || external_reply || msg.story().is_some()
}

/// Inline buttons on a message that did not come through an inline bot.
fn has_inline_keyboard(msg: &Message) -> bool {
    msg.via_bot.is_none()
        && msg
            .reply_markup()
            .is_some_and(|markup| markup.inline_keyboard.iter().any(|row| !row.is_empty()))
}

fn status_text(state: &AppState) -> String {
    let settings = &state.settings;
    let threshold = settings
        .threshold
        .unwrap_or_else(|| state.shield().model().threshold);
    let allowed = match &settings.allowed_chats {
        None => "all chats".to_string(),
        Some(set) => format!("{} chat(s)", set.len()),
    };
    let flood = &settings.flood;
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
    let trust_line = match settings.trust.after_messages {
        0 => "off (everyone is checked)".to_string(),
        n => format!(
            "after {n} clean messages of {}+ chars",
            settings.trust.min_chars
        ),
    };
    let storage = match &settings.data_dir {
        Some(dir) => dir.display().to_string(),
        None => "memory (resets on restart)".to_string(),
    };
    let on_off = |on: bool| if on { "on" } else { "off" };
    let learned = state.shield().learned_spam();
    let reports = match (settings.report_chat, settings.owner_id) {
        (Some(chat), _) => format!("chat {chat}"),
        (None, Some(_)) => "owner (private)".to_string(),
        (None, None) => "off (set ANTISPAM_OWNER_ID or ANTISPAM_REPORT_CHAT)".to_string(),
    };
    let opt_in = [
        (!settings.blocked_scripts.is_empty(), "blocked_scripts"),
        (!settings.stop_words.is_empty(), "stop_words"),
        (settings.block_forwards, "forwards"),
        (settings.link_limit > 0, "link_limit"),
        (settings.mention_limit > 0, "mention_limit"),
    ]
    .iter()
    .filter(|(on, _)| *on)
    .map(|(_, name)| *name)
    .collect::<Vec<_>>()
    .join(", ");
    format!(
        "🛡 Anti-Spam Shield\nthreshold: {threshold:.2} (model from {} chars)\naction: {:?}\n\
         dry_run: {}\njob_offers: {}\nnewcomer_links: {}\ncampaigns: {}\nrepeat_limit: {}\n\
         bot_mentions: {}\nmixed_script_words: {}\nemoji_limit: {}\nopt_in: {}\ncas: {}\n\
         learned_spam: {learned}\nwarn_limit: {}\nreports: {reports}\n\
         trust: {trust_line}\nstorage: {storage}\nexempt_admins: {}\nallowed_chats: {allowed}\n\
         leave_unknown_chats: {}\nflood: {flood_line}",
        settings.model_min_chars,
        settings.action,
        settings.dry_run,
        on_off(settings.block_job_offers),
        on_off(settings.block_newcomer_links),
        on_off(settings.block_campaigns),
        settings.repeat_limit,
        on_off(settings.block_bot_mentions),
        settings.mixed_script_words,
        settings.emoji_limit,
        if opt_in.is_empty() { "none" } else { &opt_in },
        on_off(settings.cas.enabled),
        settings.warn_limit,
        settings.exempt_admins,
        settings.leave_unknown_chats,
    )
}
