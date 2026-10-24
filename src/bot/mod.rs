//! Telegram glue: turns updates into engine calls and carries out decisions.
//!
//! The spam decision itself lives in the library ([`Shield`]); this module
//! only talks to Telegram: reading updates, deleting, banning and muting.

mod actions;
mod cas;
mod chats;
mod commands;
mod handlers;
pub mod health;
mod polling;
mod reports;

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use telegram_antispam_shield::config::Settings;
use telegram_antispam_shield::flood::FloodGuard;
use telegram_antispam_shield::shield::Shield;

pub use handlers::{on_chat_member, on_message};
pub use polling::Poller;
pub use reports::on_callback;

/// How often expired flood and history entries are dropped.
const SWEEP_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Shared application state injected into every handler.
pub struct AppState {
    pub settings: Settings,
    /// The bot's own username, to tell `/spam@ThisBot` from other bots' commands.
    bot_username: String,
    /// Held only for synchronous work, never across an `.await`.
    shield: Mutex<Shield>,
    /// Held only for synchronous checks, never across an `.await`.
    flood: Mutex<FloodGuard>,
    chats: tokio::sync::Mutex<chats::ChatCache>,
    cas: cas::Cas,
}

impl AppState {
    pub fn new(shield: Shield, bot_username: String) -> AppState {
        let settings = shield.settings().clone();
        AppState {
            bot_username,
            flood: Mutex::new(FloodGuard::new(settings.flood.clone())),
            shield: Mutex::new(shield),
            chats: tokio::sync::Mutex::new(chats::ChatCache::default()),
            cas: cas::Cas::new(settings.cas.clone()),
            settings,
        }
    }

    /// The engine. A poisoned lock only means another handler panicked; the
    /// data is still usable.
    fn shield(&self) -> MutexGuard<'_, Shield> {
        self.shield.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn flood(&self) -> MutexGuard<'_, FloodGuard> {
        self.flood.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Periodically drop expired flood and history entries to bound memory.
pub fn spawn_sweeper(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            interval.tick().await;
            let now = Instant::now();
            state.shield().sweep(now);
            let mut flood = state.flood();
            flood.sweep(now);
            tracing::debug!(tracked = flood.tracked_users(), "sweep");
        }
    });
}
