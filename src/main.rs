//! Telegram anti-spam bot entry point.
//!
//! Listens for group messages, classifies them with [`moderation::classify`],
//! and deletes spam (optionally banning/muting the author). Group admins are
//! exempt by default and their lists are cached to limit API calls.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use teloxide::prelude::*;
use teloxide::types::{ChatId, ChatPermissions, UserId};
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

use telegram_antispam_shield::config::{self, Action, Settings};
use telegram_antispam_shield::model::Model;
use telegram_antispam_shield::moderation::{classify, Verdict};

/// Per-chat cache of admin user IDs with the time they were fetched.
#[derive(Default)]
struct AdminCache {
    map: HashMap<i64, (Instant, HashSet<u64>)>,
}

/// Shared application state injected into every handler.
struct AppState {
    model: Model,
    settings: Settings,
    admins: Mutex<AdminCache>,
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
        "model_loaded"
    );

    let bot = Bot::new(config::bot_token()?);
    let state = Arc::new(AppState {
        model,
        settings,
        admins: Mutex::new(AdminCache::default()),
    });

    let handler = Update::filter_message().endpoint(on_message);
    let mut dispatcher = Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state])
        .enable_ctrlc_handler()
        .build();

    // Railway stops containers with SIGTERM — shut down gracefully.
    let shutdown = dispatcher.shutdown_token();
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

    let Some(text) = msg.text().or_else(|| msg.caption()) else {
        return Ok(());
    };
    let Some(user) = msg.from.as_ref() else {
        return Ok(()); // anonymous admin / channel post
    };
    if user.is_bot {
        return Ok(());
    }

    if state.settings.exempt_admins && is_admin(&bot, &state, msg.chat.id, user.id).await {
        return Ok(());
    }

    let Verdict::Spam { score, reason } = classify(&state.model, &state.settings, text) else {
        return Ok(());
    };

    tracing::info!(
        chat = msg.chat.id.0,
        user = user.id.0,
        score = score,
        reason = reason,
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

    match state.settings.action {
        Action::DeleteAndBan => {
            if let Err(e) = bot.ban_chat_member(msg.chat.id, user.id).await {
                tracing::warn!(error = %e, "ban_failed");
            }
        }
        Action::DeleteAndMute => {
            if let Err(e) = bot
                .restrict_chat_member(msg.chat.id, user.id, ChatPermissions::empty())
                .await
            {
                tracing::warn!(error = %e, "mute_failed");
            }
        }
        Action::DeleteOnly => {}
    }

    Ok(())
}

/// Returns whether `user` is an administrator of `chat`, using a TTL cache.
async fn is_admin(bot: &Bot, state: &AppState, chat: ChatId, user: UserId) -> bool {
    let ttl = Duration::from_secs(state.settings.admin_cache_ttl_secs);
    let now = Instant::now();
    let mut cache = state.admins.lock().await;

    let fresh = cache
        .map
        .get(&chat.0)
        .map(|(t, _)| now.duration_since(*t) < ttl)
        .unwrap_or(false);

    if !fresh {
        if let Ok(admins) = bot.get_chat_administrators(chat).await {
            let set: HashSet<u64> = admins.iter().map(|m| m.user.id.0).collect();
            cache.map.insert(chat.0, (now, set));
        }
    }

    cache
        .map
        .get(&chat.0)
        .map(|(_, s)| s.contains(&user.0))
        .unwrap_or(false)
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
    format!(
        "🛡 Anti-Spam Shield\nthreshold: {:.2}\naction: {:?}\ndry_run: {}\nexempt_admins: {}\nallowed_chats: {}\nleave_unknown_chats: {}",
        state.settings.threshold.unwrap_or(state.model.threshold),
        state.settings.action,
        state.settings.dry_run,
        state.settings.exempt_admins,
        allowed,
        state.settings.leave_unknown_chats,
    )
}
