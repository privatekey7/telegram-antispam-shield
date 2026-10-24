//! Telegram anti-spam bot entry point.
//!
//! Loads settings, the model and the optional state file, then dispatches
//! updates to [`bot`]: group messages are checked for flood and spam (and
//! admin commands), edited messages for spam, member joins feed the
//! newcomer rule, report buttons undo or confirm decisions, and private
//! chats get /start and /status.

mod bot;

use std::path::Path;
use std::sync::Arc;

use teloxide::error_handlers::LoggingErrorHandler;
use teloxide::prelude::*;
use tracing_subscriber::EnvFilter;

use telegram_antispam_shield::config::{self, Settings};
use telegram_antispam_shield::model::Model;
use telegram_antispam_shield::shield::Shield;
use telegram_antispam_shield::store::Store;

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

/// The state file in `dir`, or `None` (state in memory) if it cannot be used.
fn open_store(dir: &Path) -> Option<Store> {
    let opened = std::fs::create_dir_all(dir)
        .map_err(anyhow::Error::from)
        .and_then(|()| Store::open(dir));
    match opened {
        Ok(store) => Some(store),
        Err(e) => {
            tracing::warn!(error = %e, dir = %dir.display(), "store_unavailable (state stays in memory)");
            None
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    init_logging();

    let settings = Settings::from_env();
    let model = Model::load_default()?;
    let store = settings.data_dir.as_deref().and_then(open_store);
    tracing::info!(
        buckets = model.num_buckets,
        threshold = settings.threshold.unwrap_or(model.threshold),
        action = ?settings.action,
        dry_run = settings.dry_run,
        flood_enabled = settings.flood.enabled,
        trust_after = settings.trust.after_messages,
        persistent = store.is_some(),
        "model_loaded"
    );

    let bot = Bot::new(config::bot_token()?);
    let me = bot.get_me().await?;
    let username = me.username.clone().unwrap_or_default();
    let shield = Shield::new(model, settings, store);
    let state = Arc::new(bot::AppState::new(shield, username));
    bot::spawn_sweeper(Arc::clone(&state));

    // The handler tree also tells Telegram which update types to send.
    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(bot::on_message))
        .branch(Update::filter_edited_message().endpoint(bot::on_edited_message))
        .branch(Update::filter_chat_member().endpoint(bot::on_chat_member))
        .branch(Update::filter_callback_query().endpoint(bot::on_callback));
    let poller = bot::Poller::new(bot.clone()).await;
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
    dispatcher
        .dispatch_with_listener(
            poller,
            LoggingErrorHandler::with_custom_text("update_listener_error"),
        )
        .await;
    tracing::info!("bot_stopped");
    Ok(())
}
