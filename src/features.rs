//! Hashed text feature extraction.
//!
//! This MUST stay byte-for-byte compatible with `scripts/train_model.py`:
//! same normalization, same n-grams, same FNV-1a hash, same bucketing.
//! If you change anything here, retrain the model and update the trainer too.

use std::collections::HashSet;

/// Truncate long messages before featurization (spam is short; bounds work).
const MAX_CHARS: usize = 800;

/// 64-bit FNV-1a hash over the UTF-8 bytes of `s`.
///
/// Mirrors the Python reference implementation exactly.
#[inline]
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A "word character" is any Unicode alphanumeric (no underscore),
/// matching the Python regex `[^\W_]`.
#[inline]
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// Extract the set of feature buckets for `text`.
///
/// Features are: word unigrams, word bigrams (when `word_ngram_max >= 2`) and
/// character n-grams in `char_min..=char_max` over boundary-marked words
/// (`<word>`). Each feature string is hashed and reduced modulo `num_buckets`.
pub fn buckets(
    text: &str,
    num_buckets: u64,
    char_min: usize,
    char_max: usize,
    word_ngram_max: u8,
) -> HashSet<u64> {
    // Lower-case first, then truncate to MAX_CHARS code points (matches Python).
    let lowered: String = text.to_lowercase().chars().take(MAX_CHARS).collect();

    // Tokenize into runs of word characters.
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in lowered.chars() {
        if is_word_char(c) {
            cur.push(c);
        } else if !cur.is_empty() {
            words.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }

    let mut feats: HashSet<String> = HashSet::new();

    for (i, w) in words.iter().enumerate() {
        feats.insert(format!("w1:{w}"));
        if word_ngram_max >= 2 && i + 1 < words.len() {
            feats.insert(format!("w2:{w} {}", words[i + 1]));
        }
    }

    for w in &words {
        let cw: Vec<char> = std::iter::once('<')
            .chain(w.chars())
            .chain(std::iter::once('>'))
            .collect();
        for n in char_min..=char_max {
            if cw.len() >= n {
                for i in 0..=(cw.len() - n) {
                    let gram: String = cw[i..i + n].iter().collect();
                    feats.insert(format!("c:{gram}"));
                }
            }
        }
    }

    feats.iter().map(|f| fnv1a(f) % num_buckets).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_matches_reference() {
        // Reference values produced by the Python trainer's fnv1a().
        assert_eq!(fnv1a(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a("a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn empty_text_has_no_buckets() {
        assert!(buckets("", 1024, 3, 5, 2).is_empty());
    }

    #[test]
    fn buckets_are_within_range() {
        let nb = 1024;
        for b in buckets("Hello мир", nb, 3, 5, 2) {
            assert!(b < nb);
        }
    }
}
