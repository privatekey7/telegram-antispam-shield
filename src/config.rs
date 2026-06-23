//! Runtime settings, read once from environment variables at startup.
//!
//! Settings are intentionally separate from logic so behaviour can be tuned on
//! Railway without rebuilding (see README "Configuration").

use anyhow::{anyhow, Result};
use std::env;

/// What to do with the author of a detected spam message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Delete the message only (default, safest).
    DeleteOnly,
    /// Delete the message and ban the author.
    DeleteAndBan,
    /// Delete the message and mute (restrict) the author.
    DeleteAndMute,
}

impl Action {
    fn parse(s: &str) -> Action {
        match s.trim().to_ascii_lowercase().as_str() {
            "ban" | "delete_and_ban" | "delete+ban" => Action::DeleteAndBan,
            "mute" | "delete_and_mute" | "delete+mute" => Action::DeleteAndMute,
            _ => Action::DeleteOnly,
        }
    }
}

/// All tunable behaviour of the bot.
#[derive(Clone, Debug)]
pub struct Settings {
    pub action: Action,
    /// When true, only log detections — never delete or restrict.
    pub dry_run: bool,
    /// Optional probability threshold override (0..1); falls back to the model's.
    pub threshold: Option<f32>,
    /// Treat Telegram invite links as spam (high-precision rule).
    pub block_invite_links: bool,
    /// Never moderate group administrators.
    pub exempt_admins: bool,
    /// How long to cache a chat's admin list, in seconds.
    pub admin_cache_ttl_secs: u64,
}

fn env_bool(key: &str, default: bool) -> bool {
    match env::var(key) {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => default,
    }
}

impl Settings {
    pub fn from_env() -> Self {
        Settings {
            action: Action::parse(&env::var("ANTISPAM_ACTION").unwrap_or_default()),
            dry_run: env_bool("ANTISPAM_DRY_RUN", false),
            threshold: env::var("ANTISPAM_THRESHOLD")
                .ok()
                .and_then(|v| v.trim().parse::<f32>().ok())
                .filter(|t| (0.0..=1.0).contains(t)),
            block_invite_links: env_bool("ANTISPAM_BLOCK_INVITE_LINKS", true),
            exempt_admins: env_bool("ANTISPAM_EXEMPT_ADMINS", true),
            admin_cache_ttl_secs: env::var("ANTISPAM_ADMIN_CACHE_TTL")
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok())
                .unwrap_or(3600),
        }
    }
}

/// The bot token, from `BOT_TOKEN` (preferred) or `TELOXIDE_TOKEN`.
pub fn bot_token() -> Result<String> {
    env::var("BOT_TOKEN")
        .or_else(|_| env::var("TELOXIDE_TOKEN"))
        .map_err(|_| anyhow!("BOT_TOKEN is required (get it from @BotFather)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_parsing() {
        assert_eq!(Action::parse("ban"), Action::DeleteAndBan);
        assert_eq!(Action::parse("MUTE"), Action::DeleteAndMute);
        assert_eq!(Action::parse(""), Action::DeleteOnly);
        assert_eq!(Action::parse("whatever"), Action::DeleteOnly);
    }
}
