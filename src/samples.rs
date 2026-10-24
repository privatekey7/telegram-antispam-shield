//! Learned spam and ham samples: messages admins marked with `/spam` or
//! "not spam".
//!
//! A new message is compared with every sample by the cosine similarity of
//! their word sets, like tg-spam's similarity check. With the default
//! threshold of 0.5, spam samples taken from a real chat matched 92% of its
//! spam and none of 10,490 normal messages. So one `/spam` stops a whole
//! wave of the same promo, reworded or not.
//!
//! Samples belong to the chat they were learned in and apply only there: an
//! admin of one chat must not steer moderation in another (ADR 0011). Their
//! size and number are capped so that no amount of `/spam` can exhaust
//! memory or slow matching down.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};

use crate::store::{SampleKind, Store};
use crate::text;

/// Only the start of a text is learned or compared, as in the model
/// (`features.rs`). Telegram allows 4,096 chars; spam fits in far fewer.
const MAX_CHARS: usize = 800;
/// Distinct words kept per text. 800 chars of normal prose hold about 110–130
/// words, so the cap only bites on word salad (two-letter words reach 266).
/// In memory a word is one `u64`: a sample costs at most about 1 KB.
pub const MAX_WORDS: usize = 128;
/// Samples kept per chat and kind; the chat's oldest are dropped first.
pub const MAX_SAMPLES_PER_CHAT: usize = 1000;
/// Samples kept per kind across all chats. When full, the chat that adds one
/// drops its own oldest; only its first sample is taken from the chat holding
/// the most, so other chats cannot shrink a chat's samples.
///
/// Worst case: 2 kinds × 5,000 × ~1.1 KB ≈ 11 MB, well inside the
/// container's 128 MB. A message is compared with at most 1,000 samples of
/// each kind, each a merge of two sorted lists of ≤ 128 words: about half a
/// million integer comparisons, around a millisecond at worst (before this
/// cap, 5,000 unbounded samples took about 0.3 s per message).
pub const MAX_SAMPLES_TOTAL: usize = 5000;
/// Messages with fewer distinct words are too short to compare.
const MIN_WORDS: usize = 3;

/// Distinct lower-case words of two or more letters or digits, in order of
/// first appearance, from the first [`MAX_CHARS`] chars, at most
/// [`MAX_WORDS`] of them.
fn words(text: &str) -> Vec<String> {
    let start: String = text::strip_invisible(text)
        .chars()
        .take(MAX_CHARS)
        .collect();
    let mut seen = HashSet::new();
    start
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 2)
        .map(str::to_lowercase)
        .filter(|w| seen.insert(w.clone()))
        .take(MAX_WORDS)
        .collect()
}

/// Sorted word hashes: a compact word set that compares by merging.
/// 64-bit hashes of at most a few million distinct words practically never
/// collide, and a collision would only nudge one similarity score.
fn fingerprint(words: &[String]) -> Vec<u64> {
    let mut hashes: Vec<u64> = words
        .iter()
        .map(|w| {
            let mut hasher = DefaultHasher::new();
            w.hash(&mut hasher);
            hasher.finish()
        })
        .collect();
    hashes.sort_unstable();
    hashes.dedup();
    hashes
}

fn cosine(a: &[u64], b: &[u64]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let (mut i, mut j, mut shared) = (0, 0, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                shared += 1;
                i += 1;
                j += 1;
            }
        }
    }
    shared as f32 / ((a.len() * b.len()) as f32).sqrt()
}

/// One learned sample.
struct Sample {
    /// Row in the store, to delete it when the sample is dropped.
    id: Option<i64>,
    words: Vec<u64>,
}

/// Spam and ham samples per chat, mirrored to the [`Store`] if any.
pub struct Samples {
    threshold: f32,
    /// Oldest first.
    lists: HashMap<(i64, SampleKind), VecDeque<Sample>>,
    store: Option<Store>,
}

impl Samples {
    /// Samples loaded from `store`, or none without one. Stored rows that no
    /// longer fit the limits are deleted.
    pub fn new(threshold: f32, store: Option<Store>) -> Samples {
        let mut samples = Samples {
            threshold,
            lists: HashMap::new(),
            store,
        };
        for kind in [SampleKind::Spam, SampleKind::Ham] {
            samples.load(kind);
        }
        samples
    }

    /// Number of learned spam samples in all chats.
    pub fn spam_count(&self) -> usize {
        self.count(SampleKind::Spam)
    }

    /// Whether `text` resembles a spam sample learned in `chat`.
    pub fn resembles_spam(&self, chat: i64, text: &str) -> bool {
        self.resembles(chat, SampleKind::Spam, text)
    }

    /// Whether `text` resembles a message an admin of `chat` marked as not
    /// spam.
    pub fn resembles_ham(&self, chat: i64, text: &str) -> bool {
        self.resembles(chat, SampleKind::Ham, text)
    }

    /// Learn `text` as an example of `kind` in `chat`. Too-short texts and
    /// texts with the same words as a sample already kept are ignored.
    pub fn learn(&mut self, chat: i64, kind: SampleKind, text: &str) {
        let mut words = words(text);
        let sample = fingerprint(&words);
        if sample.len() < MIN_WORDS || self.contains(chat, kind, &sample) {
            return;
        }
        // Sorted, so the same word set is stored as the same text.
        words.sort_unstable();
        let id = self.store.as_ref().and_then(|store| {
            store
                .add_sample(chat, kind, &words.join(" "))
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "sample_save_failed");
                    None
                })
        });
        self.push(chat, kind, Sample { id, words: sample });
    }

    fn load(&mut self, kind: SampleKind) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let rows = store.samples(kind).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "samples_load_failed");
            Vec::new()
        });
        for row in rows {
            let sample = fingerprint(&words(&row.text));
            if sample.len() < MIN_WORDS || self.contains(row.chat_id, kind, &sample) {
                self.forget([Some(row.id)]);
                continue;
            }
            let sample = Sample {
                id: Some(row.id),
                words: sample,
            };
            self.push(row.chat_id, kind, sample);
        }
    }

    fn count(&self, kind: SampleKind) -> usize {
        self.lists
            .iter()
            .filter(|((_, k), _)| *k == kind)
            .map(|(_, list)| list.len())
            .sum()
    }

    fn contains(&self, chat: i64, kind: SampleKind, words: &[u64]) -> bool {
        self.lists
            .get(&(chat, kind))
            .is_some_and(|list| list.iter().any(|s| s.words == words))
    }

    /// Add `sample` as the newest, then drop samples over the limits.
    fn push(&mut self, chat: i64, kind: SampleKind, sample: Sample) {
        let list = self.lists.entry((chat, kind)).or_default();
        list.push_back(sample);
        let mut dropped = Vec::new();
        if list.len() > MAX_SAMPLES_PER_CHAT {
            dropped.extend(list.pop_front());
        }
        if self.count(kind) > MAX_SAMPLES_TOTAL {
            // The chat that overflows the total pays with its own oldest
            // sample, so no chat can push out another chat's samples: without
            // an allowlist anyone can add the bot to groups of their own. Only
            // a chat learning its very first sample takes one from the
            // largest chat, so a new chat can still learn; an attacker gets
            // one such eviction per group, not one per `/spam`.
            let own = (chat, kind);
            let key = if self.lists.get(&own).is_some_and(|l| l.len() > 1) {
                Some(own)
            } else {
                self.lists
                    .iter()
                    .filter(|((_, k), _)| *k == kind)
                    .max_by_key(|(_, list)| list.len())
                    .map(|(key, _)| *key)
            };
            if let Some(key) = key {
                let list = self.lists.get_mut(&key).expect("key was just found");
                dropped.extend(list.pop_front());
                if list.is_empty() {
                    self.lists.remove(&key);
                }
            }
        }
        self.forget(dropped.into_iter().map(|s| s.id));
    }

    /// Delete dropped samples from the store.
    fn forget(&self, ids: impl IntoIterator<Item = Option<i64>>) {
        let Some(store) = &self.store else {
            return;
        };
        for id in ids.into_iter().flatten() {
            if let Err(e) = store.remove_sample(id) {
                tracing::warn!(error = %e, "sample_delete_failed");
            }
        }
    }

    fn resembles(&self, chat: i64, kind: SampleKind, text: &str) -> bool {
        let Some(samples) = self.lists.get(&(chat, kind)) else {
            return false;
        };
        let message = fingerprint(&words(text));
        message.len() >= MIN_WORDS
            && samples
                .iter()
                .any(|sample| cosine(&message, &sample.words) >= self.threshold)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAT: i64 = -100;
    const OTHER_CHAT: i64 = -200;
    const PROMO: &str = "Ondo | апдейт и насыпка $USDC. Сегодня команда открыла клейм $USDC \
                         для активных пользователей проекта. Подключаем кошелёк и забираем";

    /// A text of three words no other `distinct(j)` shares.
    fn distinct(i: usize) -> String {
        format!("alpha{i} beta{i} gamma{i}")
    }

    #[test]
    fn reworded_promo_matches_a_learned_sample() {
        let mut samples = Samples::new(0.5, None);
        samples.learn(CHAT, SampleKind::Spam, PROMO);

        let variant = "Ondo | доступен клейм $USDC. Сегодня команда открыла клейм для \
                       пользователей проекта. После подключения кошелька забираем";
        assert!(samples.resembles_spam(CHAT, variant));
        assert!(!samples.resembles_spam(CHAT, "Кто-нибудь знает, когда Ondo запустит свой мост?"));
    }

    #[test]
    fn short_texts_are_neither_learned_nor_matched() {
        let mut samples = Samples::new(0.5, None);
        samples.learn(CHAT, SampleKind::Spam, "купи");
        assert_eq!(samples.spam_count(), 0);
        samples.learn(CHAT, SampleKind::Spam, PROMO);
        assert!(!samples.resembles_spam(CHAT, "клейм"));
    }

    #[test]
    fn a_sample_learned_in_one_chat_does_not_match_in_another() {
        let mut samples = Samples::new(0.5, None);

        samples.learn(CHAT, SampleKind::Spam, PROMO);
        samples.learn(CHAT, SampleKind::Ham, PROMO);

        assert!(samples.resembles_spam(CHAT, PROMO));
        assert!(!samples.resembles_spam(OTHER_CHAT, PROMO));
        assert!(!samples.resembles_ham(OTHER_CHAT, PROMO));
    }

    #[test]
    fn the_same_words_are_stored_once() {
        let store = Store::in_memory().unwrap();
        let mut samples = Samples::new(0.5, Some(store.clone()));
        let shuffled = "забираем КОШЕЛЁК подключаем проекта пользователей активных для \
                        $USDC клейм открыла команда Сегодня насыпка и апдейт Ondo!!!";

        samples.learn(CHAT, SampleKind::Spam, PROMO);
        samples.learn(CHAT, SampleKind::Spam, PROMO);
        samples.learn(CHAT, SampleKind::Spam, shuffled);

        assert_eq!(samples.spam_count(), 1);
        assert_eq!(store.samples(SampleKind::Spam).unwrap().len(), 1);
    }

    #[test]
    fn long_texts_are_cut_to_the_word_and_char_limits() {
        let store = Store::in_memory().unwrap();
        let mut samples = Samples::new(0.5, Some(store.clone()));
        let word_salad: String = (0..300).map(|i| format!("w{i} ")).collect();
        let padded = format!("{}купите крипту сейчас дешево", "ок ".repeat(300));

        samples.learn(CHAT, SampleKind::Spam, &word_salad);
        samples.learn(OTHER_CHAT, SampleKind::Spam, &padded);

        let stored = store.samples(SampleKind::Spam).unwrap();
        assert_eq!(stored.len(), 1, "the padded text has one word in reach");
        assert_eq!(stored[0].text.split(' ').count(), MAX_WORDS);
        assert!(samples.resembles_spam(CHAT, &word_salad));
    }

    #[test]
    fn the_per_chat_cap_drops_the_oldest() {
        let store = Store::in_memory().unwrap();
        let mut samples = Samples::new(0.5, Some(store.clone()));
        samples.learn(OTHER_CHAT, SampleKind::Spam, PROMO);

        for i in 0..=MAX_SAMPLES_PER_CHAT {
            samples.learn(CHAT, SampleKind::Spam, &distinct(i));
        }

        assert_eq!(samples.spam_count(), MAX_SAMPLES_PER_CHAT + 1);
        assert!(!samples.resembles_spam(CHAT, &distinct(0)));
        assert!(samples.resembles_spam(CHAT, &distinct(1)));
        assert!(samples.resembles_spam(CHAT, &distinct(MAX_SAMPLES_PER_CHAT)));
        assert!(samples.resembles_spam(OTHER_CHAT, PROMO));
        assert_eq!(
            store.samples(SampleKind::Spam).unwrap().len(),
            MAX_SAMPLES_PER_CHAT + 1
        );
    }

    #[test]
    fn the_total_cap_drops_from_the_largest_chat() {
        let mut samples = Samples::new(0.5, None);
        let full_chats = MAX_SAMPLES_TOTAL / MAX_SAMPLES_PER_CHAT;
        for chat in 0..full_chats {
            for i in 0..MAX_SAMPLES_PER_CHAT {
                samples.learn(chat as i64, SampleKind::Spam, &distinct(i));
            }
        }

        samples.learn(CHAT, SampleKind::Spam, PROMO);

        assert_eq!(samples.spam_count(), MAX_SAMPLES_TOTAL);
        assert!(samples.resembles_spam(CHAT, PROMO));
    }

    #[test]
    fn other_chats_cannot_shrink_a_chat_once_the_total_is_full() {
        // Arrange: our chat holds 600 samples, hostile chats fill the rest.
        let mut samples = Samples::new(0.5, None);
        for i in 0..600 {
            samples.learn(CHAT, SampleKind::Spam, &distinct(i));
        }
        let hostile = |n: usize| 1_000 + n as i64;
        let filler = (MAX_SAMPLES_TOTAL - 600) / 100;
        for n in 0..filler {
            for i in 0..100 {
                samples.learn(hostile(n), SampleKind::Spam, &distinct(i));
            }
        }
        assert_eq!(samples.spam_count(), MAX_SAMPLES_TOTAL);

        // Act: six more hostile chats with 100 `/spam` each (the audit's attack).
        for n in filler..filler + 6 {
            for i in 0..100 {
                samples.learn(hostile(n), SampleKind::Spam, &distinct(i));
            }
        }

        // Assert: one eviction per new chat, not one per sample.
        let ours = samples.lists[&(CHAT, SampleKind::Spam)].len();
        assert_eq!(ours, 600 - 6);
        assert_eq!(samples.spam_count(), MAX_SAMPLES_TOTAL);
    }

    #[test]
    fn samples_survive_a_restart() {
        let store = Store::in_memory().unwrap();
        Samples::new(0.5, Some(store.clone())).learn(CHAT, SampleKind::Ham, PROMO);

        let reloaded = Samples::new(0.5, Some(store));

        assert!(reloaded.resembles_ham(CHAT, PROMO));
        assert!(!reloaded.resembles_ham(OTHER_CHAT, PROMO));
        assert!(!reloaded.resembles_spam(CHAT, PROMO));
    }

    #[test]
    fn stored_rows_over_the_limits_are_dropped_on_load() {
        let store = Store::in_memory().unwrap();
        store.add_sample(CHAT, SampleKind::Spam, PROMO).unwrap();
        store
            .add_sample(CHAT, SampleKind::Spam, &PROMO.to_uppercase())
            .unwrap();
        store.add_sample(CHAT, SampleKind::Spam, "купи").unwrap();

        let samples = Samples::new(0.5, Some(store.clone()));

        assert_eq!(samples.spam_count(), 1);
        assert_eq!(store.samples(SampleKind::Spam).unwrap().len(), 1);
    }
}
