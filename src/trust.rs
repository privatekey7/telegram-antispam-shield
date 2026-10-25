//! Trusted members: who has earned a lighter check.
//!
//! Spam comes from fresh accounts, while regulars post links and slang that
//! a text model can misread. So, like tg-spam's "approved users", a member is
//! trusted after a few clean messages and then skips the content checks
//! (model, job offers, newcomer links). Rules with no false positives in
//! practice (buttons, hidden characters, invite links, repeats) still apply.
//!
//! Trust fades: a member silent for longer than `expires_after` is checked
//! like anyone else until a few clean messages earn it back, so a dormant or
//! sold account cannot spend old trust. Records nobody needs any more are
//! pruned (see [`Trust::prune`]).

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::store::{MemberRecord, Store};

/// Untrusted members silent this long are forgotten: their few clean
/// messages are not worth keeping.
const FORGET_UNTRUSTED_SECS: i64 = 30 * 24 * 60 * 60;
/// A trusted member's activity is written to the store at most this often.
const ACTIVITY_WRITE_SECS: i64 = 24 * 60 * 60;

/// When a member becomes trusted, and for how long.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustConfig {
    /// Clean messages needed to be trusted; 0 turns trust off (check everyone).
    pub after_messages: u32,
    /// Shorter messages ("+", "hi") do not count, so trust cannot be farmed.
    pub min_chars: usize,
    /// Trust lapses after this long without a clean message; `None` = never.
    pub expires_after: Option<Duration>,
}

impl Default for TrustConfig {
    fn default() -> Self {
        TrustConfig {
            after_messages: 3,
            min_chars: 10,
            expires_after: Some(Duration::from_secs(90 * 24 * 60 * 60)),
        }
    }
}

/// Unix time in seconds.
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Member standing per `(chat, sender)`, mirrored to the [`Store`] if any.
pub struct Trust {
    config: TrustConfig,
    members: HashMap<(i64, i64), MemberRecord>,
    store: Option<Store>,
    clock: fn() -> i64,
}

impl Trust {
    /// Trust state loaded from `store`, or empty and in-memory without one.
    pub fn new(config: TrustConfig, store: Option<Store>) -> Trust {
        Trust::with_clock(config, store, unix_now)
    }

    fn with_clock(config: TrustConfig, store: Option<Store>, clock: fn() -> i64) -> Trust {
        let members = match &store {
            Some(store) => store.members().unwrap_or_else(|e| {
                tracing::warn!(error = %e, "trust_load_failed");
                Vec::new()
            }),
            None => Vec::new(),
        };
        let mut trust = Trust {
            config,
            members: members.into_iter().collect(),
            store,
            clock,
        };
        trust.prune();
        trust
    }

    pub fn config(&self) -> &TrustConfig {
        &self.config
    }

    /// Whether `sender` is trusted in `chat` right now.
    pub fn is_trusted(&self, chat: i64, sender: i64) -> bool {
        self.config.after_messages > 0
            && self
                .members
                .get(&(chat, sender))
                .is_some_and(|m| m.trusted && !self.lapsed(m, (self.clock)()))
    }

    /// Clean messages counted so far for `sender` in `chat`.
    pub fn clean_messages(&self, chat: i64, sender: i64) -> u32 {
        self.members
            .get(&(chat, sender))
            .map_or(0, |m| m.clean_messages)
    }

    /// Count a message that passed every check. Returns `true` when this
    /// message made the sender trusted.
    pub fn record_clean(&mut self, chat: i64, sender: i64, text_chars: usize) -> bool {
        if self.config.after_messages == 0 || text_chars < self.config.min_chars {
            return false;
        }
        let now = (self.clock)();
        let expires_after = self.config.expires_after;
        let after_messages = self.config.after_messages;
        let record = self.members.entry((chat, sender)).or_default();
        if record.trusted && !lapsed(record, now, expires_after) {
            // Still trusted: only keep the activity fresh, cheaply.
            if now - record.active_at < ACTIVITY_WRITE_SECS {
                return false;
            }
            record.active_at = now;
            let record = *record;
            self.persist(chat, sender, record);
            return false;
        }
        if record.trusted {
            // Lapsed: earn it again from zero.
            *record = MemberRecord::default();
        }
        record.clean_messages += 1;
        record.trusted = record.clean_messages >= after_messages;
        record.active_at = now;
        let (record, newly_trusted) = (*record, record.trusted);
        self.persist(chat, sender, record);
        newly_trusted
    }

    /// Make `sender` trusted now (an admin vouched for them).
    pub fn approve(&mut self, chat: i64, sender: i64) {
        let now = (self.clock)();
        let record = self.members.entry((chat, sender)).or_default();
        record.trusted = true;
        record.active_at = now;
        let record = *record;
        self.persist(chat, sender, record);
    }

    /// Withdraw trust and start counting from zero (the sender sent spam).
    pub fn revoke(&mut self, chat: i64, sender: i64) {
        let Some(record) = self.members.get_mut(&(chat, sender)) else {
            return;
        };
        *record = MemberRecord::default();
        self.persist(chat, sender, MemberRecord::default());
    }

    /// Forget untrusted members silent for a month and trusted ones whose
    /// trust has lapsed, here and in the store.
    pub fn prune(&mut self) {
        let now = (self.clock)();
        let untrusted_before = now - FORGET_UNTRUSTED_SECS;
        let trusted_before = self
            .config
            .expires_after
            .map(|ttl| now - ttl.as_secs() as i64);
        self.members.retain(|_, m| {
            let cutoff = if m.trusted {
                trusted_before
            } else {
                Some(untrusted_before)
            };
            cutoff.is_none_or(|before| m.active_at >= before)
        });
        if let Some(store) = &self.store {
            if let Err(e) = store.prune_members(untrusted_before, trusted_before) {
                tracing::warn!(error = %e, "trust_prune_failed");
            }
        }
    }

    fn lapsed(&self, record: &MemberRecord, now: i64) -> bool {
        lapsed(record, now, self.config.expires_after)
    }

    fn persist(&self, chat: i64, sender: i64, record: MemberRecord) {
        if let Some(store) = &self.store {
            if let Err(e) = store.save_member(chat, sender, record) {
                tracing::warn!(error = %e, "trust_save_failed");
            }
        }
    }
}

fn lapsed(record: &MemberRecord, now: i64, expires_after: Option<Duration>) -> bool {
    expires_after.is_some_and(|ttl| now - record.active_at > ttl.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    const CHAT: i64 = -100;
    const USER: i64 = 42;
    const LONG: usize = 20;
    const DAY: i64 = 24 * 60 * 60;

    fn trust() -> Trust {
        Trust::new(TrustConfig::default(), None)
    }

    #[test]
    fn trusted_after_enough_clean_messages() {
        let mut trust = trust();

        assert!(!trust.record_clean(CHAT, USER, LONG));
        assert!(!trust.record_clean(CHAT, USER, LONG));
        assert!(!trust.is_trusted(CHAT, USER));
        assert!(trust.record_clean(CHAT, USER, LONG));

        assert!(trust.is_trusted(CHAT, USER));
        assert!(!trust.is_trusted(CHAT + 1, USER), "trust is per chat");
    }

    #[test]
    fn short_messages_do_not_count() {
        let mut trust = trust();
        for _ in 0..10 {
            trust.record_clean(CHAT, USER, 1);
        }
        assert_eq!(trust.clean_messages(CHAT, USER), 0);
    }

    #[test]
    fn spam_resets_trust() {
        let mut trust = trust();
        trust.approve(CHAT, USER);

        trust.revoke(CHAT, USER);

        assert!(!trust.is_trusted(CHAT, USER));
        assert_eq!(trust.clean_messages(CHAT, USER), 0);
    }

    #[test]
    fn zero_threshold_trusts_nobody() {
        let config = TrustConfig {
            after_messages: 0,
            ..TrustConfig::default()
        };
        let mut trust = Trust::new(config, None);
        trust.approve(CHAT, USER);
        assert!(!trust.is_trusted(CHAT, USER));
    }

    #[test]
    fn trust_survives_a_restart() {
        let store = Store::in_memory().unwrap();
        let mut trust = Trust::new(TrustConfig::default(), Some(store));
        trust.approve(CHAT, USER);

        let store = trust.store.take().unwrap();
        let reloaded = Trust::new(TrustConfig::default(), Some(store));

        assert!(reloaded.is_trusted(CHAT, USER));
    }

    static NOW: AtomicI64 = AtomicI64::new(1_800_000_000);

    fn fake_now() -> i64 {
        NOW.load(Ordering::SeqCst)
    }

    #[test]
    fn trust_lapses_after_a_long_silence_and_can_be_earned_again() {
        // Arrange: one test owns the fake clock.
        let start = 1_800_000_000;
        NOW.store(start, Ordering::SeqCst);
        let store = Store::in_memory().unwrap();
        let mut trust = Trust::with_clock(TrustConfig::default(), Some(store.clone()), fake_now);
        trust.approve(CHAT, USER);
        trust.record_clean(CHAT, USER + 1, LONG);

        // Act: 91 days of silence.
        NOW.store(start + 91 * DAY, Ordering::SeqCst);
        let after_silence = trust.is_trusted(CHAT, USER);
        let earned_back: Vec<bool> = (0..3)
            .map(|_| trust.record_clean(CHAT, USER, LONG))
            .collect();
        let reloaded = Trust::with_clock(TrustConfig::default(), Some(store), fake_now);

        // Assert
        assert!(!after_silence);
        assert_eq!(earned_back, vec![false, false, true]);
        assert!(trust.is_trusted(CHAT, USER));
        assert!(reloaded.is_trusted(CHAT, USER));
        assert_eq!(
            reloaded.clean_messages(CHAT, USER + 1),
            0,
            "an untrusted member silent for a month is forgotten"
        );
    }
}
