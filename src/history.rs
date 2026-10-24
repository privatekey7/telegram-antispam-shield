//! Recent chat events: messages and joins, kept for a day in memory.
//!
//! One message at a time looks harmless; the history shows what it repeats.
//! It powers three rules and one clean-up:
//!
//! * repeats — the same text from one sender several times a day (a bot
//!   posting every 20 minutes slips under the anti-flood window);
//! * campaigns — the same long text from different accounts (a spam wave
//!   where every account posts once);
//! * newcomers — when a member joined, for the "no links on day one" rule;
//! * clean-up — a banned spammer's other recent messages.
//!
//! Like [`crate::flood`], it is pure: callers pass `now`.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::text;

/// Shortest normalized text the repeat rule compares ("спасибо" is fine to
/// say five times a day).
pub const REPEAT_MIN_CHARS: usize = 15;
/// Shortest normalized text the campaign rule compares, so two newcomers
/// saying "всем привет" are not a campaign.
pub const CAMPAIGN_MIN_CHARS: usize = 40;
/// Messages kept per chat, so a busy chat cannot grow memory without bound.
const MAX_MESSAGES_PER_CHAT: usize = 5000;

#[derive(Debug)]
struct Entry {
    at: Instant,
    sender: i64,
    message_id: i32,
    /// Hash of the normalized text and its length in characters.
    fingerprint: Option<(u64, usize)>,
}

#[derive(Debug, Default)]
struct ChatLog {
    messages: VecDeque<Entry>,
    joins: HashMap<i64, Instant>,
}

/// Earlier messages with the same text as the one just recorded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Echoes {
    /// Earlier copies from the same sender (long enough to compare).
    pub repeats: Vec<i32>,
    /// Copies from other senders, as `(sender, message_id)`, when the text is
    /// long enough to be a campaign.
    pub others: Vec<(i64, i32)>,
}

/// Messages and joins of every chat within `window`.
#[derive(Debug)]
pub struct History {
    window: Duration,
    chats: HashMap<i64, ChatLog>,
}

impl History {
    pub fn new(window: Duration) -> History {
        History {
            window,
            chats: HashMap::new(),
        }
    }

    /// Remember a message and return its echoes among earlier messages.
    pub fn record(
        &mut self,
        chat: i64,
        sender: i64,
        message_id: i32,
        text: Option<&str>,
        now: Instant,
    ) -> Echoes {
        let window = self.window;
        let log = self.chats.entry(chat).or_default();
        prune(&mut log.messages, window, now);

        let fingerprint = fingerprint(text);
        let echoes = find_echoes(log.messages.iter(), sender, fingerprint);

        if log.messages.len() >= MAX_MESSAGES_PER_CHAT {
            log.messages.pop_front();
        }
        log.messages.push_back(Entry {
            at: now,
            sender,
            message_id,
            fingerprint,
        });
        echoes
    }

    /// Echoes of an edited message's new text, without recording it: an edit
    /// is not a new message. The message's own earlier version is skipped.
    pub fn echoes_of_edit(
        &self,
        chat: i64,
        sender: i64,
        message_id: i32,
        text: Option<&str>,
        now: Instant,
    ) -> Echoes {
        let Some(log) = self.chats.get(&chat) else {
            return Echoes::default();
        };
        let earlier = log.messages.iter().filter(|m| {
            now.duration_since(m.at) <= self.window
                && !(m.sender == sender && m.message_id == message_id)
        });
        find_echoes(earlier, sender, fingerprint(text))
    }

    /// Remember that `user` joined `chat`.
    pub fn joined(&mut self, chat: i64, user: i64, now: Instant) {
        self.chats.entry(chat).or_default().joins.insert(user, now);
    }

    /// Whether `sender` joined `chat` within `within` (and the bot saw it).
    pub fn joined_within(&self, chat: i64, sender: i64, within: Duration, now: Instant) -> bool {
        self.chats
            .get(&chat)
            .and_then(|log| log.joins.get(&sender))
            .is_some_and(|at| now.duration_since(*at) < within)
    }

    /// IDs of `sender`'s messages still in the history.
    pub fn recent_from(&self, chat: i64, sender: i64) -> Vec<i32> {
        self.chats.get(&chat).map_or_else(Vec::new, |log| {
            log.messages
                .iter()
                .filter(|m| m.sender == sender)
                .map(|m| m.message_id)
                .collect()
        })
    }

    /// Drop expired messages and joins; call periodically.
    pub fn sweep(&mut self, now: Instant) {
        let window = self.window;
        self.chats.retain(|_, log| {
            prune(&mut log.messages, window, now);
            log.joins.retain(|_, at| now.duration_since(*at) < window);
            !log.messages.is_empty() || !log.joins.is_empty()
        });
    }
}

/// Hash of the normalized text and its length in characters, if the text is
/// long enough to compare.
fn fingerprint(text: Option<&str>) -> Option<(u64, usize)> {
    text.and_then(|t| {
        let chars = text::normalize(t).chars().count();
        text::fingerprint(t, REPEAT_MIN_CHARS).map(|hash| (hash, chars))
    })
}

/// Messages among `earlier` with the same text as `fingerprint`.
fn find_echoes<'a>(
    earlier: impl Iterator<Item = &'a Entry>,
    sender: i64,
    fingerprint: Option<(u64, usize)>,
) -> Echoes {
    let mut echoes = Echoes::default();
    let Some((hash, chars)) = fingerprint else {
        return echoes;
    };
    for entry in earlier {
        if entry.fingerprint.map(|(h, _)| h) != Some(hash) {
            continue;
        }
        if entry.sender == sender {
            echoes.repeats.push(entry.message_id);
        } else if chars >= CAMPAIGN_MIN_CHARS {
            echoes.others.push((entry.sender, entry.message_id));
        }
    }
    echoes
}

fn prune(messages: &mut VecDeque<Entry>, window: Duration, now: Instant) {
    while messages
        .front()
        .is_some_and(|m| now.duration_since(m.at) > window)
    {
        messages.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAT: i64 = -100;
    const DAY: Duration = Duration::from_secs(24 * 60 * 60);
    const PROMO: &str = "Ondo | доступен клейм $USDC для активных пользователей проекта";

    fn mins(n: u64) -> Duration {
        Duration::from_secs(n * 60)
    }

    #[test]
    fn repeats_from_one_sender_are_found_hours_apart() {
        let mut history = History::new(DAY);
        let start = Instant::now();
        let spam = "有 至副堡 VX来 帮我收米一天赚几k @lpbxya";

        history.record(CHAT, 1, 10, Some(spam), start);
        history.record(CHAT, 1, 11, Some(spam), start + mins(20));
        let echoes = history.record(CHAT, 1, 12, Some(spam), start + mins(40));

        assert_eq!(echoes.repeats, vec![10, 11]);
        assert!(echoes.others.is_empty());
    }

    #[test]
    fn short_messages_are_not_repeats() {
        let mut history = History::new(DAY);
        let start = Instant::now();
        history.record(CHAT, 1, 10, Some("спасибо!"), start);
        let echoes = history.record(CHAT, 1, 11, Some("Спасибо"), start + mins(5));
        assert!(echoes.repeats.is_empty());
    }

    #[test]
    fn same_long_text_from_other_accounts_is_a_campaign() {
        let mut history = History::new(DAY);
        let start = Instant::now();

        history.record(CHAT, 1, 10, Some(PROMO), start);
        let echoes = history.record(CHAT, 2, 20, Some(PROMO), start + mins(90));

        assert_eq!(echoes.others, vec![(1, 10)]);
    }

    #[test]
    fn short_greetings_from_newcomers_are_not_a_campaign() {
        let mut history = History::new(DAY);
        let start = Instant::now();
        history.record(CHAT, 1, 10, Some("Всем привет, рад быть тут!"), start);
        let echoes = history.record(CHAT, 2, 20, Some("всем привет рад быть тут"), start);
        assert!(echoes.others.is_empty());
    }

    #[test]
    fn old_messages_expire() {
        let mut history = History::new(DAY);
        let start = Instant::now();
        history.record(CHAT, 1, 10, Some(PROMO), start);
        let echoes = history.record(CHAT, 2, 20, Some(PROMO), start + DAY + mins(1));
        assert!(echoes.others.is_empty());
    }

    #[test]
    fn chats_are_separate() {
        let mut history = History::new(DAY);
        let start = Instant::now();
        history.record(CHAT, 1, 10, Some(PROMO), start);
        let echoes = history.record(CHAT - 1, 2, 20, Some(PROMO), start);
        assert!(echoes.others.is_empty());
    }

    #[test]
    fn joins_are_remembered_for_the_window() {
        let mut history = History::new(DAY);
        let start = Instant::now();
        history.joined(CHAT, 7, start);

        assert!(history.joined_within(CHAT, 7, DAY, start + mins(60)));
        assert!(!history.joined_within(CHAT, 7, mins(30), start + mins(60)));
        assert!(!history.joined_within(CHAT, 8, DAY, start));
    }

    #[test]
    fn recent_messages_of_a_sender_are_listed() {
        let mut history = History::new(DAY);
        let start = Instant::now();
        history.record(CHAT, 1, 10, Some("первое"), start);
        history.record(CHAT, 2, 11, Some("чужое"), start);
        history.record(CHAT, 1, 12, None, start);

        assert_eq!(history.recent_from(CHAT, 1), vec![10, 12]);
    }

    #[test]
    fn sweep_drops_expired_chats() {
        let mut history = History::new(DAY);
        let start = Instant::now();
        history.record(CHAT, 1, 10, Some(PROMO), start);
        history.joined(CHAT, 2, start);

        history.sweep(start + DAY + mins(1));

        assert!(history.chats.is_empty());
    }
}
