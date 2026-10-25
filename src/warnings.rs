//! Admin warnings (`/warn`): a per-member counter that fades after a quiet
//! month, mirrored to the [`Store`] if any.

use std::collections::HashMap;

use crate::store::{Store, WarningRecord};

/// Warnings older than this (since the last one) no longer count.
pub const WARNING_TTL_SECS: i64 = 30 * 24 * 60 * 60;

pub struct Warnings {
    members: HashMap<(i64, i64), WarningRecord>,
    store: Option<Store>,
}

impl Warnings {
    pub fn new(store: Option<Store>) -> Warnings {
        let members = store.as_ref().map_or_else(Vec::new, |s| {
            s.warnings().unwrap_or_else(|e| {
                tracing::warn!(error = %e, "warnings_load_failed");
                Vec::new()
            })
        });
        let mut warnings = Warnings {
            members: members.into_iter().collect(),
            store,
        };
        warnings.prune(crate::trust::unix_now());
        warnings
    }

    /// Forget warnings that no longer count at `now` (Unix seconds).
    pub fn prune(&mut self, now: i64) {
        let before = now - WARNING_TTL_SECS;
        self.members
            .retain(|_, record| record.count > 0 && record.updated_at >= before);
        if let Some(store) = &self.store {
            if let Err(e) = store.prune_warnings(before) {
                tracing::warn!(error = %e, "warnings_prune_failed");
            }
        }
    }

    /// Record a warning at `now` (Unix seconds) and return how many count.
    pub fn warn(&mut self, chat: i64, user: i64, now: i64) -> u32 {
        let record = self.members.entry((chat, user)).or_default();
        if now - record.updated_at > WARNING_TTL_SECS {
            record.count = 0;
        }
        record.count += 1;
        record.updated_at = now;
        let record = *record;
        self.persist(chat, user, record);
        record.count
    }

    /// Forget a member's warnings (after a ban, or an admin's change of mind).
    pub fn clear(&mut self, chat: i64, user: i64) {
        if self.members.remove(&(chat, user)).is_some() {
            self.persist(chat, user, WarningRecord::default());
        }
    }

    fn persist(&self, chat: i64, user: i64, record: WarningRecord) {
        if let Some(store) = &self.store {
            if let Err(e) = store.save_warning(chat, user, record) {
                tracing::warn!(error = %e, "warning_save_failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn warnings_add_up_and_fade_after_a_quiet_month() {
        let mut warnings = Warnings::new(None);

        assert_eq!(warnings.warn(-1, 5, NOW), 1);
        assert_eq!(warnings.warn(-1, 5, NOW + 60), 2);
        assert_eq!(warnings.warn(-2, 5, NOW), 1, "per chat");
        assert_eq!(warnings.warn(-1, 5, NOW + 60 + WARNING_TTL_SECS + 1), 1);
    }

    #[test]
    fn clearing_starts_over() {
        let mut warnings = Warnings::new(None);
        warnings.warn(-1, 5, NOW);
        warnings.clear(-1, 5);
        assert_eq!(warnings.warn(-1, 5, NOW), 1);
    }

    #[test]
    fn faded_warnings_are_forgotten() {
        let mut warnings = Warnings::new(None);
        warnings.warn(-1, 5, NOW);

        warnings.prune(NOW + WARNING_TTL_SECS + 1);

        assert!(warnings.members.is_empty());
    }

    #[test]
    fn warnings_survive_a_restart() {
        let store = Store::in_memory().unwrap();
        Warnings::new(Some(store.clone())).warn(-1, 5, NOW);
        assert_eq!(Warnings::new(Some(store)).warn(-1, 5, NOW + 1), 2);
    }
}
