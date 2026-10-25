//! Keeps one busy chat from stalling all the others.
//!
//! teloxide gives every chat its own worker with a bounded queue, and when a
//! queue is full the dispatcher waits, which stops updates for every chat. A
//! raid in one group (or a stranger's group, or a slow API) would then delay
//! moderation everywhere.
//!
//! So the poller counts the updates it has handed over per chat and drops an
//! update instead of handing it over when its chat's queue is full, or when
//! too many updates wait in total (memory). The handler tree marks each
//! update as taken when a worker starts on it. A dropped update goes
//! unmoderated, which is the lesser harm: the alternative was every chat
//! waiting.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

/// Queue size of each chat's worker (teloxide's `worker_queue_size`).
pub const CHAT_QUEUE: usize = 256;
/// Updates waiting in all queues together; past this, new ones are dropped.
const TOTAL_QUEUED: usize = 4096;

#[derive(Default)]
struct ChatBacklog {
    queued: usize,
    /// Dropped since the queue last ran empty (logged once it does).
    dropped: usize,
}

#[derive(Default)]
struct Counts {
    chats: HashMap<i64, ChatBacklog>,
    total: usize,
    /// Dropped because of [`TOTAL_QUEUED`] from chats with nothing queued
    /// (they get no entry, so the map stays bounded).
    dropped_over_total: usize,
}

/// Updates handed to the dispatcher and not yet taken by a worker.
#[derive(Default)]
pub struct Backlog {
    counts: Mutex<Counts>,
}

impl Backlog {
    /// Whether an update for `chat` may be handed over now; counts it if so.
    pub fn admit(&self, chat: i64) -> bool {
        let mut counts = self.counts();
        let total_full = counts.total >= TOTAL_QUEUED;
        let Some(entry) = counts.chats.get_mut(&chat) else {
            if total_full {
                counts.dropped_over_total += 1;
                if counts.dropped_over_total == 1 {
                    tracing::warn!(chat, total = TOTAL_QUEUED, "backlog_total_full");
                }
                return false;
            }
            counts.chats.insert(
                chat,
                ChatBacklog {
                    queued: 1,
                    dropped: 0,
                },
            );
            counts.total += 1;
            return true;
        };
        if total_full || entry.queued >= CHAT_QUEUE {
            entry.dropped += 1;
            if entry.dropped == 1 {
                tracing::warn!(chat, queued = entry.queued, "chat_backlog_full");
            }
            return false;
        }
        entry.queued += 1;
        counts.total += 1;
        true
    }

    /// A worker took an update for `chat` off its queue.
    pub fn taken(&self, chat: i64) {
        let mut counts = self.counts();
        let Some(entry) = counts.chats.get_mut(&chat) else {
            return;
        };
        entry.queued = entry.queued.saturating_sub(1);
        if entry.queued == 0 {
            if entry.dropped > 0 {
                tracing::warn!(chat, dropped = entry.dropped, "chat_backlog_recovered");
            }
            counts.chats.remove(&chat);
        }
        counts.total = counts.total.saturating_sub(1);
        if counts.dropped_over_total > 0 && counts.total < TOTAL_QUEUED {
            tracing::warn!(
                dropped = counts.dropped_over_total,
                "backlog_total_recovered"
            );
            counts.dropped_over_total = 0;
        }
    }

    /// A poisoned lock only means a panic elsewhere; the counts still hold.
    fn counts(&self) -> MutexGuard<'_, Counts> {
        self.counts.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_chat_drops_its_own_updates_only() {
        // Arrange
        let backlog = Backlog::default();
        for _ in 0..CHAT_QUEUE {
            assert!(backlog.admit(-1));
        }

        // Act
        let busy = backlog.admit(-1);
        let other = backlog.admit(-2);

        // Assert
        assert!(!busy);
        assert!(other);
    }

    #[test]
    fn a_taken_update_frees_a_place() {
        // Arrange
        let backlog = Backlog::default();
        for _ in 0..CHAT_QUEUE {
            backlog.admit(-1);
        }

        // Act
        backlog.taken(-1);

        // Assert
        assert!(backlog.admit(-1));
        assert!(!backlog.admit(-1));
    }

    #[test]
    fn the_total_is_capped_and_empty_chats_are_forgotten() {
        // Arrange: many chats, each below its own limit.
        let backlog = Backlog::default();
        let chats = (TOTAL_QUEUED / CHAT_QUEUE) as i64;
        for chat in 0..chats {
            for _ in 0..CHAT_QUEUE {
                assert!(backlog.admit(chat));
            }
        }

        // Act
        let over_total = backlog.admit(-999);
        for _ in 0..CHAT_QUEUE {
            backlog.taken(0);
        }
        let after_drain = backlog.admit(-999);

        // Assert
        assert!(!over_total);
        assert!(after_drain);
        assert_eq!(
            backlog.counts().chats.len(),
            chats as usize,
            "no entry for drops"
        );
        assert!(!backlog.counts().chats.contains_key(&0));
    }
}
