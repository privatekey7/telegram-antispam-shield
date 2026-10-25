//! Runtime settings, read once from environment variables at startup.
//!
//! Settings are intentionally separate from logic so behaviour can be tuned on
//! any host without rebuilding (see README "Configuration").

use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::env;
use std::path::PathBuf;
use std::time::Duration;

use crate::flood::FloodConfig;
use crate::meta::Script;
use crate::trust::TrustConfig;

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
    /// Allowlist of chat IDs the bot will operate in. `None` = every chat,
    /// which [`Settings::from_env`] allows only with `ANTISPAM_ALLOW_ALL_CHATS`;
    /// an empty set (no list yet) = no chat.
    pub allowed_chats: Option<HashSet<i64>>,
    /// When true, leave any chat that is not in `allowed_chats` on first sight.
    pub leave_unknown_chats: bool,
    /// Optional owner user ID; when set, only this user gets `/status` in private.
    pub owner_id: Option<i64>,
    /// Anti-flood rules (message rate and repeats).
    pub flood: FloodConfig,
    /// When members become trusted (content checks are skipped for them).
    pub trust: TrustConfig,
    /// How long messages and joins are remembered for the repeat, campaign
    /// and newcomer rules.
    pub history_window: Duration,
    /// Spam when one sender posts the same text this many times within
    /// `history_window`; 0 turns the rule off.
    pub repeat_limit: usize,
    /// Treat the same long text from several untrusted accounts as spam.
    pub block_campaigns: bool,
    /// Treat links from members who joined within `history_window` and have
    /// not written a clean message yet as spam.
    pub block_newcomer_links: bool,
    /// When banning or muting a spammer, also delete their other messages
    /// from the history window.
    pub cleanup_on_ban: bool,
    /// Directory for the SQLite state file; `None` keeps state in memory.
    pub data_dir: Option<PathBuf>,
    /// Shorter messages without links skip the model: too little signal.
    pub model_min_chars: usize,
    /// Spam when this many words mix Cyrillic and Latin letters; 0 = off.
    pub mixed_script_words: usize,
    /// Spam when a message has this many emoji; 0 = off.
    pub emoji_limit: usize,
    /// Treat a message that is only a bot's `@username` as spam.
    pub block_bot_mentions: bool,
    /// Writing systems whose messages are spam (opt-in).
    pub blocked_scripts: Vec<Script>,
    /// Lower-case phrases that make a message spam (opt-in).
    pub stop_words: Vec<String>,
    /// Treat forwards and quotes from other chats as spam (opt-in).
    pub block_forwards: bool,
    /// Spam when a message has this many links; 0 = off.
    pub link_limit: usize,
    /// Spam when a message has this many `@mentions`; 0 = off.
    pub mention_limit: usize,
    /// Delete "joined" / "left" service messages.
    pub delete_service_messages: bool,
    /// Cosine similarity at which a message matches a learned sample.
    pub similarity_threshold: f32,
    /// Ban a member after this many admin warnings; 0 = never.
    pub warn_limit: u32,
    /// Chat that receives spam reports with "not spam" buttons; falls back
    /// to a private message to the owner.
    pub report_chat: Option<i64>,
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
            trust: TrustConfig::default(),
            history_window: Duration::from_secs(24 * 60 * 60),
            repeat_limit: 3,
            block_campaigns: true,
            block_newcomer_links: true,
            cleanup_on_ban: true,
            data_dir: None,
            model_min_chars: 20,
            mixed_script_words: 2,
            emoji_limit: 10,
            block_bot_mentions: true,
            blocked_scripts: Vec::new(),
            stop_words: Vec::new(),
            block_forwards: false,
            link_limit: 0,
            mention_limit: 0,
            delete_service_messages: false,
            similarity_threshold: 0.5,
            warn_limit: 3,
            report_chat: None,
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

    /// Where spam reports go: the report chat, else the owner, else nowhere.
    pub fn report_destination(&self) -> Option<i64> {
        self.report_chat.or(self.owner_id)
    }

    /// Whether `user_id` is the configured owner, who may see `/status`.
    /// With no owner configured, nobody is.
    pub fn is_owner(&self, user_id: i64) -> bool {
        self.owner_id == Some(user_id)
    }

    /// Risky but valid choices worth a warning at startup.
    pub fn access_warnings(&self) -> Vec<&'static str> {
        let mut warnings = Vec::new();
        match &self.allowed_chats {
            None => warnings.push(
                "ANTISPAM_ALLOW_ALL_CHATS is on: anyone can add the bot to a group, \
                 use your hosting, send you reports and fill the bot's total sample \
                 budget. Prefer ANTISPAM_ALLOWED_CHATS with your group IDs.",
            ),
            Some(chats) if chats.is_empty() => {
                warnings.push(
                    "ANTISPAM_ALLOWED_CHATS is not set: the bot moderates no chat yet. \
                     Add it to your group and write there; the group's ID appears in \
                     this log as chat_not_allowed. Put it in ANTISPAM_ALLOWED_CHATS \
                     and restart.",
                );
                if self.leave_unknown_chats {
                    warnings.push(
                        "ANTISPAM_LEAVE_UNKNOWN_CHATS is on with no allowlist: the bot \
                         leaves every group it sees.",
                    );
                }
            }
            Some(_) => {}
        }
        if self.owner_id.is_none() {
            warnings.push("ANTISPAM_OWNER_ID is not set: /status shows nothing to anyone.");
        }
        warnings
    }
}

fn env_secs(key: &str, default: Duration) -> Duration {
    env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(default)
}

/// A non-negative number where 0 is meaningful (usually "off").
fn env_number<T: std::str::FromStr>(key: &str, default: T) -> T {
    env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<T>().ok())
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
    /// Read the settings. Access control fails closed: without
    /// `ANTISPAM_ALLOWED_CHATS` (or an explicit `ANTISPAM_ALLOW_ALL_CHATS`)
    /// the bot serves no chat, and a typo in it or in `ANTISPAM_OWNER_ID`
    /// stops the bot with an error.
    pub fn from_env() -> Result<Self> {
        let d = Settings::default();
        Ok(Settings {
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
            admin_cache_ttl_secs: env_number("ANTISPAM_ADMIN_CACHE_TTL", d.admin_cache_ttl_secs),
            allowed_chats: resolve_allowlist(
                env::var("ANTISPAM_ALLOWED_CHATS").ok(),
                env_bool("ANTISPAM_ALLOW_ALL_CHATS", false),
            )?,
            leave_unknown_chats: env_bool("ANTISPAM_LEAVE_UNKNOWN_CHATS", d.leave_unknown_chats),
            owner_id: parse_owner_id(env::var("ANTISPAM_OWNER_ID").ok())?,
            flood: flood_from_env(),
            trust: TrustConfig {
                after_messages: env_number("ANTISPAM_TRUST_AFTER", d.trust.after_messages),
                min_chars: env_number("ANTISPAM_TRUST_MIN_CHARS", d.trust.min_chars),
                expires_after: trust_ttl(env::var("ANTISPAM_TRUST_TTL_DAYS").ok())
                    .unwrap_or(d.trust.expires_after),
            },
            history_window: env_secs("ANTISPAM_HISTORY_WINDOW_SECS", d.history_window),
            repeat_limit: env_number("ANTISPAM_REPEAT_LIMIT", d.repeat_limit),
            block_campaigns: env_bool("ANTISPAM_BLOCK_CAMPAIGNS", d.block_campaigns),
            block_newcomer_links: env_bool("ANTISPAM_BLOCK_NEWCOMER_LINKS", d.block_newcomer_links),
            cleanup_on_ban: env_bool("ANTISPAM_CLEANUP_ON_BAN", d.cleanup_on_ban),
            data_dir: env::var("ANTISPAM_DATA_DIR")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .map(PathBuf::from),
            model_min_chars: env_number("ANTISPAM_MODEL_MIN_CHARS", d.model_min_chars),
            mixed_script_words: env_number("ANTISPAM_MIXED_SCRIPT_WORDS", d.mixed_script_words),
            emoji_limit: env_number("ANTISPAM_EMOJI_LIMIT", d.emoji_limit),
            block_bot_mentions: env_bool("ANTISPAM_BLOCK_BOT_MENTIONS", d.block_bot_mentions),
            blocked_scripts: parse_list(env::var("ANTISPAM_BLOCKED_SCRIPTS").ok())
                .iter()
                .filter_map(|name| Script::parse(name))
                .collect(),
            stop_words: parse_list(env::var("ANTISPAM_STOP_WORDS").ok())
                .into_iter()
                .map(|w| w.to_lowercase())
                .collect(),
            block_forwards: env_bool("ANTISPAM_BLOCK_FORWARDS", d.block_forwards),
            link_limit: env_number("ANTISPAM_LINK_LIMIT", d.link_limit),
            mention_limit: env_number("ANTISPAM_MENTION_LIMIT", d.mention_limit),
            delete_service_messages: env_bool(
                "ANTISPAM_DELETE_SERVICE_MESSAGES",
                d.delete_service_messages,
            ),
            similarity_threshold: env::var("ANTISPAM_SIMILARITY_THRESHOLD")
                .ok()
                .and_then(|v| v.trim().parse::<f32>().ok())
                .filter(|t| (0.0..=1.0).contains(t))
                .unwrap_or(d.similarity_threshold),
            warn_limit: env_number("ANTISPAM_WARN_LIMIT", d.warn_limit),
            report_chat: env::var("ANTISPAM_REPORT_CHAT")
                .ok()
                .and_then(|v| v.trim().parse::<i64>().ok()),
        })
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

/// Parse a list of phrases separated by commas or newlines (spaces are kept,
/// so "stop word" can be a phrase).
fn parse_list(raw: Option<String>) -> Vec<String> {
    raw.unwrap_or_default()
        .split([',', '\n'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `ANTISPAM_TRUST_TTL_DAYS`: `0` = trust never lapses; `None` when unset or
/// not a number (the default applies).
fn trust_ttl(raw: Option<String>) -> Option<Option<Duration>> {
    let days = raw?.trim().parse::<u64>().ok()?;
    Some((days > 0).then(|| Duration::from_secs(days.saturating_mul(24 * 60 * 60))))
}

/// The allowlist from `ANTISPAM_ALLOWED_CHATS`. Without one the bot would
/// serve any group a stranger adds it to, so an unset or blank list allows
/// no chat (the bot only logs each group's ID, to be added) unless
/// `allow_all` says every chat is intended.
fn resolve_allowlist(raw: Option<String>, allow_all: bool) -> Result<Option<HashSet<i64>>> {
    match parse_id_set(raw)? {
        Some(set) => Ok(Some(set)),
        None if allow_all => Ok(None),
        None => Ok(Some(HashSet::new())),
    }
}

/// Parse a list of chat IDs separated by commas, spaces, semicolons or newlines.
/// Returns `None` when unset or blank, meaning "all chats allowed". Any entry
/// that is not a number (`@mygroup`, a Unicode minus) is an error, so a typo
/// cannot open the bot to every chat.
fn parse_id_set(raw: Option<String>) -> Result<Option<HashSet<i64>>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let set = raw
        .split([',', ' ', ';', '\n', '\t', '\r'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            p.parse::<i64>().map_err(|_| {
                anyhow!(
                    "ANTISPAM_ALLOWED_CHATS: {p:?} is not a numeric chat ID \
                     (groups look like -1001234567890; usernames are not accepted)"
                )
            })
        })
        .collect::<Result<HashSet<i64>>>()?;
    Ok((!set.is_empty()).then_some(set))
}

/// Parse `ANTISPAM_OWNER_ID`: unset or blank means no owner; anything else
/// must be a numeric user ID.
fn parse_owner_id(raw: Option<String>) -> Result<Option<i64>> {
    let Some(raw) = raw.filter(|v| !v.trim().is_empty()) else {
        return Ok(None);
    };
    raw.trim().parse::<i64>().map(Some).map_err(|_| {
        anyhow!(
            "ANTISPAM_OWNER_ID: {:?} is not a numeric user ID (get yours from @userinfobot)",
            raw.trim()
        )
    })
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
        // Arrange
        let raw = Some("-1001234, -1005678 ;42\r\n".to_string());

        // Act
        let parsed = parse_id_set(raw).unwrap().unwrap();

        // Assert
        assert!(parsed.contains(&-1001234));
        assert!(parsed.contains(&-1005678));
        assert!(parsed.contains(&42));
        assert_eq!(parsed.len(), 3);
    }

    #[test]
    fn unset_or_blank_allowlist_allows_every_chat() {
        // Arrange
        let unset = None;
        let blank = Some("   ".to_string());

        // Act
        let from_unset = parse_id_set(unset).unwrap();
        let from_blank = parse_id_set(blank).unwrap();

        // Assert
        assert_eq!(from_unset, None);
        assert_eq!(from_blank, None);
    }

    #[test]
    fn invalid_allowlist_entry_is_an_error() {
        // Arrange
        let typos = [
            "not_a_number",
            "@mygroup",
            "-1001234,@mygroup",
            "\u{2212}1001234", // Unicode minus sign
        ];

        for raw in typos {
            // Act
            let parsed = parse_id_set(Some(raw.to_string()));

            // Assert
            let error = parsed.expect_err(raw).to_string();
            assert!(error.contains("ANTISPAM_ALLOWED_CHATS"), "{error}");
        }
    }

    #[test]
    fn owner_id_parsing() {
        // Arrange
        let valid = Some(" 777 ".to_string());
        let blank = Some(String::new());
        let invalid = Some("@me".to_string());

        // Act
        let from_valid = parse_owner_id(valid).unwrap();
        let from_blank = parse_owner_id(blank).unwrap();
        let from_unset = parse_owner_id(None).unwrap();
        let from_invalid = parse_owner_id(invalid);

        // Assert
        assert_eq!(from_valid, Some(777));
        assert_eq!(from_blank, None);
        assert_eq!(from_unset, None);
        assert!(from_invalid
            .unwrap_err()
            .to_string()
            .contains("ANTISPAM_OWNER_ID"));
    }

    #[test]
    fn open_access_is_warned_about() {
        // Arrange
        let open = Settings::default();
        let closed = Settings {
            allowed_chats: Some([-1001234].into_iter().collect()),
            owner_id: Some(777),
            ..Settings::default()
        };

        // Act
        let open_warnings = open.access_warnings();
        let closed_warnings = closed.access_warnings();

        // Assert
        assert_eq!(open_warnings.len(), 2);
        assert!(open_warnings[0].contains("ANTISPAM_ALLOW_ALL_CHATS"));
        assert!(closed_warnings.is_empty());
    }

    #[test]
    fn a_missing_allowlist_allows_no_chat_unless_all_chats_are_allowed() {
        // Arrange
        let listed = Some("-1001234".to_string());

        // Act
        let unset = resolve_allowlist(None, false).unwrap();
        let blank = resolve_allowlist(Some(" ".to_string()), false).unwrap();
        let all = resolve_allowlist(None, true).unwrap();
        let with_list = resolve_allowlist(listed, true).unwrap();

        // Assert
        assert_eq!(unset, Some(HashSet::new()));
        assert_eq!(blank, Some(HashSet::new()));
        assert_eq!(all, None);
        assert_eq!(with_list, Some([-1001234].into_iter().collect()));
        let no_list = Settings {
            allowed_chats: unset,
            ..Settings::default()
        };
        assert!(!no_list.chat_allowed(-1001234));
        assert!(no_list.access_warnings()[0].contains("chat_not_allowed"));
    }

    #[test]
    fn trust_ttl_parsing() {
        assert_eq!(
            trust_ttl(Some("30".into())),
            Some(Some(Duration::from_secs(30 * 24 * 60 * 60)))
        );
        assert_eq!(trust_ttl(Some("0".into())), Some(None), "0 = never lapses");
        assert_eq!(trust_ttl(Some("soon".into())), None);
        assert_eq!(trust_ttl(None), None);
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
    fn phrase_list_parsing() {
        assert_eq!(
            parse_list(Some(" казино, быстрый заработок ,\nxxx".into())),
            vec!["казино", "быстрый заработок", "xxx"]
        );
        assert!(parse_list(None).is_empty());
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
    fn without_an_owner_nobody_is_the_owner() {
        // Arrange
        let s = Settings {
            owner_id: None,
            ..Settings::default()
        };

        // Act
        let stranger = s.is_owner(777);

        // Assert
        assert!(!stranger);
    }

    #[test]
    fn with_an_owner_only_that_user_is_the_owner() {
        // Arrange
        let s = Settings {
            owner_id: Some(777),
            ..Settings::default()
        };

        // Act
        let owner = s.is_owner(777);
        let stranger = s.is_owner(778);

        // Assert
        assert!(owner);
        assert!(!stranger);
    }
}
