//! Runtime settings, read once from environment variables at startup.
//!
//! Settings are intentionally separate from logic so behaviour can be tuned on
//! any host without rebuilding (see README "Configuration").

use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::env;
use std::time::Duration;

use crate::flood::FloodConfig;

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
    /// Treat job and income offers with pay or "DM me" as spam.
    pub block_job_offers: bool,
    /// Treat invisible characters inside words (`act\u{200B}ive`) as spam.
    pub block_hidden_characters: bool,
    /// Treat inline buttons on a person's message as spam (only automation,
    /// such as a business bot, can attach them).
    pub block_inline_keyboards: bool,
    /// Never moderate group administrators.
    pub exempt_admins: bool,
    /// How long to cache a chat's admin list, in seconds.
    pub admin_cache_ttl_secs: u64,
    /// Allowlist of chat IDs the bot will operate in. `None` = every chat
    /// (backward-compatible default). When `Some`, only listed chats are moderated.
    pub allowed_chats: Option<HashSet<i64>>,
    /// When true, leave any chat that is not in `allowed_chats` on first sight.
    pub leave_unknown_chats: bool,
    /// Optional owner user ID; when set, only this user gets `/status` in private.
    pub owner_id: Option<i64>,
    /// Anti-flood rules (message rate and repeats).
    pub flood: FloodConfig,
}

impl Default for Settings {
    /// The documented defaults (see README "Configuration").
    fn default() -> Self {
        Settings {
            action: Action::DeleteOnly,
            dry_run: false,
            threshold: None,
            block_invite_links: true,
            block_job_offers: true,
            block_hidden_characters: true,
            block_inline_keyboards: true,
            exempt_admins: true,
            admin_cache_ttl_secs: 3600,
            allowed_chats: None,
            leave_unknown_chats: false,
            owner_id: None,
            flood: FloodConfig::default(),
        }
    }
}

impl Settings {
    /// Whether the bot is permitted to operate in `chat_id`.
    /// With no allowlist configured, every chat is allowed.
    pub fn chat_allowed(&self, chat_id: i64) -> bool {
        match &self.allowed_chats {
            None => true,
            Some(set) => set.contains(&chat_id),
        }
    }

    /// Whether `user_id` may see private-chat status output.
    /// With no owner configured, anyone may.
    pub fn is_owner(&self, user_id: i64) -> bool {
        match self.owner_id {
            None => true,
            Some(owner) => owner == user_id,
        }
    }
}

fn env_secs(key: &str, default: Duration) -> Duration {
    env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(default)
}

fn env_count(key: &str, default: usize) -> usize {
    env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
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
        let d = Settings::default();
        Settings {
            action: Action::parse(&env::var("ANTISPAM_ACTION").unwrap_or_default()),
            dry_run: env_bool("ANTISPAM_DRY_RUN", d.dry_run),
            threshold: env::var("ANTISPAM_THRESHOLD")
                .ok()
                .and_then(|v| v.trim().parse::<f32>().ok())
                .filter(|t| (0.0..=1.0).contains(t)),
            block_invite_links: env_bool("ANTISPAM_BLOCK_INVITE_LINKS", d.block_invite_links),
            block_job_offers: env_bool("ANTISPAM_BLOCK_JOB_OFFERS", d.block_job_offers),
            block_hidden_characters: env_bool(
                "ANTISPAM_BLOCK_HIDDEN_CHARACTERS",
                d.block_hidden_characters,
            ),
            block_inline_keyboards: env_bool(
                "ANTISPAM_BLOCK_INLINE_KEYBOARDS",
                d.block_inline_keyboards,
            ),
            exempt_admins: env_bool("ANTISPAM_EXEMPT_ADMINS", d.exempt_admins),
            admin_cache_ttl_secs: env::var("ANTISPAM_ADMIN_CACHE_TTL")
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok())
                .unwrap_or(d.admin_cache_ttl_secs),
            allowed_chats: parse_id_set(env::var("ANTISPAM_ALLOWED_CHATS").ok()),
            leave_unknown_chats: env_bool("ANTISPAM_LEAVE_UNKNOWN_CHATS", d.leave_unknown_chats),
            owner_id: env::var("ANTISPAM_OWNER_ID")
                .ok()
                .and_then(|v| v.trim().parse::<i64>().ok()),
            flood: flood_from_env(),
        }
    }
}

fn flood_from_env() -> FloodConfig {
    let defaults = FloodConfig::default();
    FloodConfig {
        enabled: env_bool("ANTISPAM_FLOOD_ENABLED", defaults.enabled),
        max_messages: env_count("ANTISPAM_FLOOD_MAX_MESSAGES", defaults.max_messages),
        rate_window: env_secs("ANTISPAM_FLOOD_WINDOW_SECS", defaults.rate_window),
        max_duplicates: env_count("ANTISPAM_FLOOD_MAX_DUPLICATES", defaults.max_duplicates),
        duplicate_window: env_secs(
            "ANTISPAM_FLOOD_DUPLICATE_WINDOW_SECS",
            defaults.duplicate_window,
        ),
        mute_steps: env::var("ANTISPAM_FLOOD_MUTE_STEPS")
            .ok()
            .and_then(|v| parse_duration_list(&v))
            .unwrap_or(defaults.mute_steps),
        strike_reset: env_secs("ANTISPAM_FLOOD_STRIKE_RESET_SECS", defaults.strike_reset),
        notify: env_bool("ANTISPAM_FLOOD_NOTIFY", defaults.notify),
        notify_ttl: env_secs("ANTISPAM_FLOOD_NOTIFY_TTL_SECS", defaults.notify_ttl),
    }
}

/// Parse durations like `10m,1h,1d` (suffixes `s`, `m`, `h`, `d`; bare numbers
/// are seconds). Returns `None` if any item is invalid or the list is empty.
fn parse_duration_list(raw: &str) -> Option<Vec<Duration>> {
    let steps: Option<Vec<Duration>> = raw
        .split([',', ' ', ';'])
        .filter(|p| !p.trim().is_empty())
        .map(|p| parse_duration(p.trim()))
        .collect();
    steps.filter(|s| !s.is_empty())
}

fn parse_duration(raw: &str) -> Option<Duration> {
    let (number, multiplier) = match raw.char_indices().last()? {
        (i, 's') => (&raw[..i], 1),
        (i, 'm') => (&raw[..i], 60),
        (i, 'h') => (&raw[..i], 60 * 60),
        (i, 'd') => (&raw[..i], 24 * 60 * 60),
        _ => (raw, 1),
    };
    let value = number.trim().parse::<u64>().ok().filter(|v| *v > 0)?;
    Some(Duration::from_secs(value.checked_mul(multiplier)?))
}

/// Parse a list of chat IDs separated by commas, spaces, semicolons or newlines.
/// Returns `None` when unset or empty, meaning "all chats allowed".
fn parse_id_set(raw: Option<String>) -> Option<HashSet<i64>> {
    let raw = raw?;
    let set: HashSet<i64> = raw
        .split([',', ' ', ';', '\n', '\t'])
        .filter_map(|p| p.trim().parse::<i64>().ok())
        .collect();
    if set.is_empty() {
        None
    } else {
        Some(set)
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

    #[test]
    fn id_set_parsing() {
        assert_eq!(parse_id_set(None), None);
        assert_eq!(parse_id_set(Some("   ".into())), None);
        assert_eq!(parse_id_set(Some("not_a_number".into())), None);

        let parsed = parse_id_set(Some("-1001234, -1005678 ;42".into())).unwrap();
        assert!(parsed.contains(&-1001234));
        assert!(parsed.contains(&-1005678));
        assert!(parsed.contains(&42));
        assert_eq!(parsed.len(), 3);
    }

    #[test]
    fn duration_list_parsing() {
        let parsed = parse_duration_list("10m, 1h,1d;90").unwrap();
        assert_eq!(
            parsed,
            vec![
                Duration::from_secs(600),
                Duration::from_secs(3600),
                Duration::from_secs(86400),
                Duration::from_secs(90),
            ]
        );
        assert_eq!(parse_duration_list(""), None);
        assert_eq!(parse_duration_list("10m,soon"), None);
        assert_eq!(parse_duration_list("0m"), None);
    }

    #[test]
    fn chat_allowlist_logic() {
        // No allowlist => every chat allowed.
        let mut s = Settings {
            allowed_chats: None,
            ..Settings::default()
        };
        assert!(s.chat_allowed(-1001234));

        // With an allowlist => only listed chats.
        s.allowed_chats = Some([-1001234].into_iter().collect());
        assert!(s.chat_allowed(-1001234));
        assert!(!s.chat_allowed(-9999999));
    }

    #[test]
    fn owner_logic() {
        // No owner => anyone is "owner" for status.
        let mut s = Settings {
            owner_id: None,
            ..Settings::default()
        };
        assert!(s.is_owner(777));

        // With an owner => only that user.
        s.owner_id = Some(777);
        assert!(s.is_owner(777));
        assert!(!s.is_owner(778));
    }
}
