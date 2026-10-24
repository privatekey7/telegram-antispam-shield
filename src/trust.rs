//! Trusted members: who has earned a lighter check.
//!
//! Spam comes from fresh accounts, while regulars post links and slang that
//! a text model can misread. So, like tg-spam's "approved users", a member is
//! trusted after a few clean messages and then skips the content checks
//! (model, job offers, newcomer links). Rules with no false positives in
//! practice (buttons, hidden characters, invite links, repeats) still apply.

use std::collections::HashMap;

use crate::store::{MemberRecord, Store};

/// When a member becomes trusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustConfig {
    /// Clean messages needed to be trusted; 0 turns trust off (check everyone).
    pub after_messages: u32,
    /// Shorter messages ("+", "hi") do not count, so trust cannot be farmed.
    pub min_chars: usize,
}

impl Default for TrustConfig {
    fn default() -> Self {
        TrustConfig {
            after_messages: 3,
            min_chars: 10,
        }
    }
}

/// Member standing per `(chat, sender)`, mirrored to the [`Store`] if any.
pub struct Trust {
    config: TrustConfig,
    members: HashMap<(i64, i64), MemberRecord>,
    store: Option<Store>,
}

impl Trust {
    /// Trust state loaded from `store`, or empty and in-memory without one.
    pub fn new(config: TrustConfig, store: Option<Store>) -> Trust {
        let members = match &store {
            Some(store) => store.members().unwrap_or_else(|e| {
                tracing::warn!(error = %e, "trust_load_failed");
                Vec::new()
            }),
            None => Vec::new(),
        };
        Trust {
            config,
            members: members.into_iter().collect(),
            store,
        }
    }

    pub fn config(&self) -> &TrustConfig {
        &self.config
    }

    /// Whether `sender` is trusted in `chat`.
    pub fn is_trusted(&self, chat: i64, sender: i64) -> bool {
        self.config.after_messages > 0
            && self.members.get(&(chat, sender)).is_some_and(|m| m.trusted)
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
        let record = self.members.entry((chat, sender)).or_default();
        if record.trusted {
            return false;
        }
        record.clean_messages += 1;
        record.trusted = record.clean_messages >= self.config.after_messages;
        let (record, newly_trusted) = (*record, record.trusted);
        self.persist(chat, sender, record);
        newly_trusted
    }

    /// Make `sender` trusted now (an admin vouched for them).
    pub fn approve(&mut self, chat: i64, sender: i64) {
        let record = self.members.entry((chat, sender)).or_default();
        record.trusted = true;
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

    fn persist(&self, chat: i64, sender: i64, record: MemberRecord) {
        if let Some(store) = &self.store {
            if let Err(e) = store.save_member(chat, sender, record) {
                tracing::warn!(error = %e, "trust_save_failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAT: i64 = -100;
    const USER: i64 = 42;
    const LONG: usize = 20;

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
}
