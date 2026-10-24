//! Per-user flood detection: message rate and repeated messages.
//!
//! The spam classifier looks at one message at a time, so ten identical
//! harmless messages look like ten harmless messages. [`FloodGuard`] keeps a
//! short sliding window of recent messages per `(chat, sender)` and flags a sender
//! who either posts too fast or repeats the same text. Repeat offenders get
//! progressively longer mutes.
//!
//! The module is pure (no Telegram, no clock): callers pass `now`, which keeps
//! every rule unit-testable.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use crate::text;

/// Shortest normalized text the duplicate rule applies to. Replies like "+"
/// or "ок" are legitimately repeated; the rate rule still covers them.
const MIN_DUPLICATE_CHARS: usize = 3;

/// Tunable flood rules (see README "Configuration").
#[derive(Clone, Debug, PartialEq)]
pub struct FloodConfig {
    /// Master switch for flood detection.
    pub enabled: bool,
    /// Flood when a user sends more than this many messages within `rate_window`.
    /// An album (media group) counts as one message.
    pub max_messages: usize,
    pub rate_window: Duration,
    /// Flood when this many messages with the same normalized text arrive
    /// within `duplicate_window`.
    pub max_duplicates: usize,
    pub duplicate_window: Duration,
    /// Mute length for the 1st, 2nd, ... offence; the last step repeats.
    pub mute_steps: Vec<Duration>,
    /// Offence counter resets after this long without a new offence.
    pub strike_reset: Duration,
    /// Post a short notice in the chat when a user is muted.
    pub notify: bool,
    /// Delete that notice after this long.
    pub notify_ttl: Duration,
}

impl Default for FloodConfig {
    fn default() -> Self {
        FloodConfig {
            enabled: true,
            max_messages: 6,
            rate_window: Duration::from_secs(10),
            max_duplicates: 3,
            duplicate_window: Duration::from_secs(120),
            mute_steps: vec![
                Duration::from_secs(10 * 60),
                Duration::from_secs(60 * 60),
                Duration::from_secs(24 * 60 * 60),
            ],
            strike_reset: Duration::from_secs(24 * 60 * 60),
            notify: true,
            notify_ttl: Duration::from_secs(30),
        }
    }
}

/// One incoming group message, reduced to what flood rules need.
#[derive(Clone, Copy, Debug)]
pub struct IncomingMessage<'a> {
    pub chat_id: i64,
    /// Telegram user ID, or the chat ID of a channel posting in the group.
    pub sender_id: i64,
    pub message_id: i32,
    /// Text or caption, if any.
    pub text: Option<&'a str>,
    /// Telegram `media_group_id` for albums.
    pub media_group_id: Option<&'a str>,
}

/// Which rule fired.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloodReason {
    /// Too many messages in the rate window.
    Rate,
    /// The same text repeated too often.
    Duplicate,
}

impl FloodReason {
    pub fn as_str(self) -> &'static str {
        match self {
            FloodReason::Rate => "flood_rate",
            FloodReason::Duplicate => "flood_duplicate",
        }
    }
}

/// The outcome of [`FloodGuard::check`].
#[derive(Clone, Debug, PartialEq)]
pub enum FloodVerdict {
    /// Nothing to do.
    Allow,
    /// The user is flooding: delete `message_ids` and mute for `mute_for`.
    Flood {
        reason: FloodReason,
        message_ids: Vec<i32>,
        mute_for: Duration,
        /// 1-based offence number within the current strike period.
        strike: u32,
    },
}

#[derive(Debug)]
struct RecentMessage {
    at: Instant,
    message_id: i32,
    fingerprint: Option<u64>,
    /// Messages sharing a key count once toward the rate limit (albums).
    rate_key: RateKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum RateKey {
    Message(i32),
    MediaGroup(String),
}

#[derive(Debug, Default)]
struct UserState {
    recent: VecDeque<RecentMessage>,
    strikes: u32,
    last_strike: Option<Instant>,
}

/// Sliding-window flood detector for all chats.
#[derive(Debug)]
pub struct FloodGuard {
    config: FloodConfig,
    users: HashMap<(i64, i64), UserState>,
}

impl FloodGuard {
    pub fn new(config: FloodConfig) -> Self {
        FloodGuard {
            config,
            users: HashMap::new(),
        }
    }

    pub fn config(&self) -> &FloodConfig {
        &self.config
    }

    /// Record `msg` and decide whether its author is flooding.
    pub fn check(&mut self, msg: IncomingMessage<'_>, now: Instant) -> FloodVerdict {
        if !self.config.enabled {
            return FloodVerdict::Allow;
        }
        let config = &self.config;
        let state = self.users.entry((msg.chat_id, msg.sender_id)).or_default();

        expire_strikes(state, config.strike_reset, now);
        prune_recent(&mut state.recent, history_window(config), now);
        state.recent.push_back(RecentMessage {
            at: now,
            message_id: msg.message_id,
            fingerprint: msg
                .text
                .and_then(|t| text::fingerprint(t, MIN_DUPLICATE_CHARS)),
            rate_key: match msg.media_group_id {
                Some(group) => RateKey::MediaGroup(group.to_owned()),
                None => RateKey::Message(msg.message_id),
            },
        });

        let Some((reason, message_ids)) = detect(&state.recent, config, now) else {
            return FloodVerdict::Allow;
        };

        state.strikes += 1;
        state.last_strike = Some(now);
        // Start over so the next few messages after a mute don't re-trigger.
        state.recent.clear();

        FloodVerdict::Flood {
            reason,
            message_ids,
            mute_for: mute_for_strike(&config.mute_steps, state.strikes),
            strike: state.strikes,
        }
    }

    /// Drop users with no recent messages and no active strikes, so memory
    /// stays bounded. Call periodically.
    pub fn sweep(&mut self, now: Instant) {
        let window = history_window(&self.config);
        let strike_reset = self.config.strike_reset;
        self.users.retain(|_, state| {
            expire_strikes(state, strike_reset, now);
            prune_recent(&mut state.recent, window, now);
            !state.recent.is_empty() || state.strikes > 0
        });
    }

    /// Number of tracked `(chat, user)` pairs.
    pub fn tracked_users(&self) -> usize {
        self.users.len()
    }
}

/// How far back messages must be kept to evaluate both rules.
fn history_window(config: &FloodConfig) -> Duration {
    config.rate_window.max(config.duplicate_window)
}

fn expire_strikes(state: &mut UserState, strike_reset: Duration, now: Instant) {
    if let Some(last) = state.last_strike {
        if now.duration_since(last) >= strike_reset {
            state.strikes = 0;
            state.last_strike = None;
        }
    }
}

fn prune_recent(recent: &mut VecDeque<RecentMessage>, window: Duration, now: Instant) {
    while recent
        .front()
        .is_some_and(|m| now.duration_since(m.at) > window)
    {
        recent.pop_front();
    }
}

/// Apply both rules to the user's recent messages (the newest is last).
/// Returns the rule that fired and the messages to delete.
fn detect(
    recent: &VecDeque<RecentMessage>,
    config: &FloodConfig,
    now: Instant,
) -> Option<(FloodReason, Vec<i32>)> {
    let within = |window: Duration| {
        recent
            .iter()
            .filter(move |m| now.duration_since(m.at) <= window)
    };

    let distinct_messages: HashSet<&RateKey> =
        within(config.rate_window).map(|m| &m.rate_key).collect();
    if distinct_messages.len() > config.max_messages {
        let ids = within(config.rate_window).map(|m| m.message_id).collect();
        return Some((FloodReason::Rate, ids));
    }

    let newest = recent.back()?.fingerprint?;
    let duplicates: Vec<i32> = within(config.duplicate_window)
        .filter(|m| m.fingerprint == Some(newest))
        .map(|m| m.message_id)
        .collect();
    if duplicates.len() >= config.max_duplicates {
        return Some((FloodReason::Duplicate, duplicates));
    }

    None
}

fn mute_for_strike(steps: &[Duration], strike: u32) -> Duration {
    let index = (strike.max(1) - 1) as usize;
    steps
        .get(index)
        .or(steps.last())
        .copied()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAT: i64 = -100;
    const USER: i64 = 42;

    fn text(message_id: i32, text: &str) -> IncomingMessage<'_> {
        IncomingMessage {
            chat_id: CHAT,
            sender_id: USER,
            message_id,
            text: Some(text),
            media_group_id: None,
        }
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn normal_conversation_is_allowed() {
        let mut guard = FloodGuard::new(FloodConfig::default());
        let start = Instant::now();
        let lines = ["привет", "как дела?", "у меня норм", "го созвон", "ок"];

        for (i, line) in lines.iter().enumerate() {
            let verdict = guard.check(text(i as i32, line), start + secs(i as u64 * 3));
            assert_eq!(verdict, FloodVerdict::Allow);
        }
    }

    #[test]
    fn third_identical_message_is_flood_and_deletes_all_copies() {
        let mut guard = FloodGuard::new(FloodConfig::default());
        let start = Instant::now();

        assert_eq!(
            guard.check(text(1, "Смотрите мой проект"), start),
            FloodVerdict::Allow
        );
        assert_eq!(
            guard.check(text(2, "смотрите мой проект!"), start + secs(20)),
            FloodVerdict::Allow
        );
        let verdict = guard.check(text(3, "СМОТРИТЕ МОЙ ПРОЕКТ"), start + secs(40));

        assert_eq!(
            verdict,
            FloodVerdict::Flood {
                reason: FloodReason::Duplicate,
                message_ids: vec![1, 2, 3],
                mute_for: secs(10 * 60),
                strike: 1,
            }
        );
    }

    #[test]
    fn duplicates_outside_window_are_allowed() {
        let mut guard = FloodGuard::new(FloodConfig::default());
        let start = Instant::now();

        for i in 0..3 {
            let verdict = guard.check(text(i, "доброе утро всем"), start + secs(i as u64 * 121));
            assert_eq!(verdict, FloodVerdict::Allow);
        }
    }

    #[test]
    fn short_replies_are_not_duplicates() {
        let mut guard = FloodGuard::new(FloodConfig::default());
        let start = Instant::now();

        for i in 0..3 {
            let verdict = guard.check(text(i, "+"), start + secs(i as u64 * 5));
            assert_eq!(verdict, FloodVerdict::Allow);
        }
    }

    #[test]
    fn seventh_message_in_ten_seconds_is_rate_flood() {
        let mut guard = FloodGuard::new(FloodConfig::default());
        let start = Instant::now();

        for i in 0..6 {
            let verdict = guard.check(text(i, &format!("сообщение {i}")), start + secs(1));
            assert_eq!(verdict, FloodVerdict::Allow);
        }
        let verdict = guard.check(text(6, "и ещё одно"), start + secs(2));

        match verdict {
            FloodVerdict::Flood {
                reason,
                message_ids,
                ..
            } => {
                assert_eq!(reason, FloodReason::Rate);
                assert_eq!(message_ids, (0..7).collect::<Vec<_>>());
            }
            FloodVerdict::Allow => panic!("7 messages in 10 s must be flood"),
        }
    }

    #[test]
    fn media_without_text_counts_toward_rate() {
        let mut guard = FloodGuard::new(FloodConfig::default());
        let start = Instant::now();
        let sticker = |id| IncomingMessage {
            text: None,
            ..text(id, "")
        };

        for i in 0..6 {
            assert_eq!(guard.check(sticker(i), start), FloodVerdict::Allow);
        }
        assert!(matches!(
            guard.check(sticker(6), start),
            FloodVerdict::Flood {
                reason: FloodReason::Rate,
                ..
            }
        ));
    }

    #[test]
    fn album_counts_as_one_message() {
        let mut guard = FloodGuard::new(FloodConfig::default());
        let start = Instant::now();

        for i in 0..10 {
            let photo = IncomingMessage {
                text: None,
                media_group_id: Some("album-1"),
                ..text(i, "")
            };
            assert_eq!(guard.check(photo, start), FloodVerdict::Allow);
        }
    }

    #[test]
    fn mute_escalates_and_resets_after_quiet_period() {
        let mut guard = FloodGuard::new(FloodConfig::default());
        let start = Instant::now();
        let mut id = 0;
        let mut flood_at = |guard: &mut FloodGuard, at: Instant| {
            let mut last = FloodVerdict::Allow;
            for _ in 0..3 {
                id += 1;
                last = guard.check(text(id, "одно и то же"), at);
            }
            last
        };

        let mutes: Vec<Duration> = [0, 60, 120]
            .iter()
            .map(|m| match flood_at(&mut guard, start + secs(m * 60)) {
                FloodVerdict::Flood { mute_for, .. } => mute_for,
                FloodVerdict::Allow => panic!("expected flood"),
            })
            .collect();
        assert_eq!(mutes, vec![secs(600), secs(3600), secs(86400)]);

        // The last step repeats.
        let fourth = flood_at(&mut guard, start + secs(180 * 60));
        assert!(
            matches!(fourth, FloodVerdict::Flood { strike: 4, mute_for, .. } if mute_for == secs(86400))
        );

        // A day without offences starts the ladder over.
        let after_quiet = flood_at(&mut guard, start + secs(180 * 60) + secs(86400));
        assert!(
            matches!(after_quiet, FloodVerdict::Flood { strike: 1, mute_for, .. } if mute_for == secs(600))
        );
    }

    #[test]
    fn users_and_chats_are_tracked_separately() {
        let mut guard = FloodGuard::new(FloodConfig::default());
        let start = Instant::now();

        for (i, sender_id) in [1, 2, 3].into_iter().enumerate() {
            let msg = IncomingMessage {
                sender_id,
                ..text(i as i32, "всем привет из чата")
            };
            assert_eq!(guard.check(msg, start), FloodVerdict::Allow);
        }
    }

    #[test]
    fn disabled_guard_allows_everything() {
        let config = FloodConfig {
            enabled: false,
            ..FloodConfig::default()
        };
        let mut guard = FloodGuard::new(config);
        let start = Instant::now();

        for i in 0..20 {
            assert_eq!(
                guard.check(text(i, "флуд флуд"), start),
                FloodVerdict::Allow
            );
        }
    }

    #[test]
    fn sweep_drops_idle_users_but_keeps_strikes() {
        let mut guard = FloodGuard::new(FloodConfig::default());
        let start = Instant::now();
        guard.check(text(1, "просто сообщение"), start);
        for i in 0..3 {
            guard.check(
                IncomingMessage {
                    sender_id: 7,
                    ..text(10 + i, "одно и то же")
                },
                start,
            );
        }

        guard.sweep(start + secs(600));
        assert_eq!(guard.tracked_users(), 1, "only the offender is remembered");

        guard.sweep(start + secs(86400));
        assert_eq!(guard.tracked_users(), 0);
    }
}
