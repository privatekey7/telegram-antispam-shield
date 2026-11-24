//! Hashed text feature extraction.
//!
//! This MUST stay byte-for-byte compatible with `scripts/train_model.py`:
//! same normalization, same n-grams, same FNV-1a hash, same bucketing.
//! If you change anything here, retrain the model and update the trainer too.

/// Truncate long messages before featurization (spam is short; bounds work).
const MAX_CHARS: usize = 800;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// 64-bit FNV-1a over the UTF-8 bytes of `s`, continuing from state `h`.
/// Hashing a feature piece by piece equals hashing the joined string, so
/// features are never built as strings.
#[inline]
fn fnv1a_from(mut h: u64, s: &str) -> u64 {
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// 64-bit FNV-1a hash over the UTF-8 bytes of `s`.
///
/// Mirrors the Python reference implementation exactly.
#[inline]
fn fnv1a(s: &str) -> u64 {
    fnv1a_from(FNV_OFFSET, s)
}

/// A "word character" is any Unicode alphanumeric (no underscore),
/// matching the Python regex `[^\W_]`.
#[inline]
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// Extract the distinct feature buckets for `text`, sorted.
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
) -> Vec<u64> {
    // Lower-case first, then truncate to MAX_CHARS code points (matches Python).
    let lowered: String = text.to_lowercase().chars().take(MAX_CHARS).collect();
    let words: Vec<&str> = lowered
        .split(|c: char| !is_word_char(c))
        .filter(|w| !w.is_empty())
        .collect();

    let mut buckets: Vec<u64> = Vec::new();
    let unigram = fnv1a("w1:");
    let bigram = fnv1a("w2:");
    let char_gram = fnv1a("c:");

    // Byte offsets of the characters of one boundary-marked word, plus its end.
    let mut marked = String::new();
    let mut offsets: Vec<usize> = Vec::new();
    for (i, w) in words.iter().enumerate() {
        buckets.push(fnv1a_from(unigram, w) % num_buckets);
        if word_ngram_max >= 2 && i + 1 < words.len() {
            let h = fnv1a_from(fnv1a_from(bigram, w), " ");
            buckets.push(fnv1a_from(h, words[i + 1]) % num_buckets);
        }

        marked.clear();
        marked.push('<');
        marked.push_str(w);
        marked.push('>');
        offsets.clear();
        offsets.extend(marked.char_indices().map(|(at, _)| at));
        let chars = offsets.len();
        offsets.push(marked.len());
        for n in char_min..=char_max.min(chars) {
            for start in 0..=(chars - n) {
                let gram = &marked[offsets[start]..offsets[start + n]];
                buckets.push(fnv1a_from(char_gram, gram) % num_buckets);
            }
        }
    }

    // The same feature (or two features in one bucket) counts once.
    buckets.sort_unstable();
    buckets.dedup();
    buckets
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
    fn piecewise_hashing_equals_hashing_the_joined_feature() {
        let h = fnv1a_from(fnv1a_from(fnv1a("w2:"), "привет"), " ");
        assert_eq!(fnv1a_from(h, "мир"), fnv1a("w2:привет мир"));
    }

    #[test]
    fn buckets_match_the_reference_feature_strings() {
        // The trainer's definition, spelled out with whole feature strings.
        let text = "Привет, МИР! hello_world 42 привет";
        let (nb, lo, hi) = (1 << 20, 2, 4);
        let lowered = text.to_lowercase();
        let words: Vec<&str> = lowered
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect();
        let mut features = std::collections::HashSet::new();
        for (i, w) in words.iter().enumerate() {
            features.insert(format!("w1:{w}"));
            if let Some(next) = words.get(i + 1) {
                features.insert(format!("w2:{w} {next}"));
            }
            let marked: Vec<char> = format!("<{w}>").chars().collect();
            for n in lo..=hi {
                for gram in marked.windows(n) {
                    features.insert(format!("c:{}", gram.iter().collect::<String>()));
                }
            }
        }
        let mut expected: Vec<u64> = features.iter().map(|f| fnv1a(f) % nb).collect();
        expected.sort_unstable();
        expected.dedup();

        assert_eq!(buckets(text, nb, lo, hi, 2), expected);
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
