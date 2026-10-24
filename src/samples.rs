//! Learned spam and ham samples: messages admins marked with `/spam` or
//! "not spam".
//!
//! A new message is compared with every sample by the cosine similarity of
//! their word sets, like tg-spam's similarity check. With the default
//! threshold of 0.5, spam samples taken from a real chat matched 92% of its
//! spam and none of 10,490 normal messages. So one `/spam` stops a whole
//! wave of the same promo, reworded or not.

use std::collections::HashSet;

use crate::store::{SampleKind, Store};
use crate::text;

/// Samples kept per kind; the oldest are dropped first.
pub const MAX_SAMPLES: usize = 5000;
/// Messages with fewer distinct words are too short to compare.
const MIN_WORDS: usize = 3;

/// Distinct lower-case words of two or more letters or digits.
fn words(text: &str) -> HashSet<String> {
    text::strip_invisible(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 2)
        .map(str::to_lowercase)
        .collect()
}

fn cosine(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let shared = a.intersection(b).count() as f32;
    shared / ((a.len() * b.len()) as f32).sqrt()
}

/// Spam and ham samples, mirrored to the [`Store`] if any.
pub struct Samples {
    threshold: f32,
    spam: Vec<HashSet<String>>,
    ham: Vec<HashSet<String>>,
    store: Option<Store>,
}

impl Samples {
    /// Samples loaded from `store`, or none without one.
    pub fn new(threshold: f32, store: Option<Store>) -> Samples {
        let load = |kind| -> Vec<HashSet<String>> {
            let texts = store.as_ref().map_or_else(Vec::new, |s| {
                s.samples(kind).unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "samples_load_failed");
                    Vec::new()
                })
            });
            texts
                .iter()
                .map(|t| words(t))
                .filter(|w| w.len() >= MIN_WORDS)
                .collect()
        };
        Samples {
            threshold,
            spam: load(SampleKind::Spam),
            ham: load(SampleKind::Ham),
            store,
        }
    }

    /// Number of learned spam samples.
    pub fn spam_count(&self) -> usize {
        self.spam.len()
    }

    /// Whether `text` resembles a learned spam sample.
    pub fn resembles_spam(&self, text: &str) -> bool {
        self.resembles(&self.spam, text)
    }

    /// Whether `text` resembles a message an admin marked as not spam.
    pub fn resembles_ham(&self, text: &str) -> bool {
        self.resembles(&self.ham, text)
    }

    /// Learn `text` as an example of `kind`. Too-short texts are ignored.
    pub fn learn(&mut self, kind: SampleKind, text: &str) {
        let sample = words(text);
        if sample.len() < MIN_WORDS {
            return;
        }
        let list = match kind {
            SampleKind::Spam => &mut self.spam,
            SampleKind::Ham => &mut self.ham,
        };
        if list.len() >= MAX_SAMPLES {
            list.remove(0);
        }
        list.push(sample);
        if let Some(store) = &self.store {
            if let Err(e) = store.add_sample(kind, text, MAX_SAMPLES) {
                tracing::warn!(error = %e, "sample_save_failed");
            }
        }
    }

    fn resembles(&self, samples: &[HashSet<String>], text: &str) -> bool {
        let message = words(text);
        message.len() >= MIN_WORDS
            && samples
                .iter()
                .any(|sample| cosine(&message, sample) >= self.threshold)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROMO: &str = "Ondo | апдейт и насыпка $USDC. Сегодня команда открыла клейм $USDC \
                         для активных пользователей проекта. Подключаем кошелёк и забираем";

    #[test]
    fn reworded_promo_matches_a_learned_sample() {
        let mut samples = Samples::new(0.5, None);
        samples.learn(SampleKind::Spam, PROMO);

        let variant = "Ondo | доступен клейм $USDC. Сегодня команда открыла клейм для \
                       пользователей проекта. После подключения кошелька забираем";
        assert!(samples.resembles_spam(variant));
        assert!(!samples.resembles_spam("Кто-нибудь знает, когда Ondo запустит свой мост?"));
    }

    #[test]
    fn short_texts_are_neither_learned_nor_matched() {
        let mut samples = Samples::new(0.5, None);
        samples.learn(SampleKind::Spam, "купи");
        assert_eq!(samples.spam_count(), 0);
        samples.learn(SampleKind::Spam, PROMO);
        assert!(!samples.resembles_spam("клейм"));
    }

    #[test]
    fn samples_survive_a_restart() {
        let store = Store::in_memory().unwrap();
        Samples::new(0.5, Some(store.clone())).learn(SampleKind::Ham, PROMO);

        let reloaded = Samples::new(0.5, Some(store));

        assert!(reloaded.resembles_ham(PROMO));
        assert!(!reloaded.resembles_spam(PROMO));
    }
}
