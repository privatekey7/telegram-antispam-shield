//! Text clean-up shared by the spam rules: invisible characters and the
//! normalized form used to spot repeated messages.
//!
//! Spammers hide words from filters with invisible characters (`act\u{200B}ive`).
//! The classifier sees the cleaned text, and an invisible character between
//! two letters is itself a strong spam signal: people do not type them.

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use unicode_normalization::UnicodeNormalization;

/// Characters that render as nothing. Zero-width (non-)joiners and the
/// direction marks are included for stripping only: they legitimately join
/// emoji and some scripts, or order right-to-left text. The Hangul fillers
/// are blank "letters" that modern Korean text does not use.
fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{115F}'
            | '\u{1160}'
            | '\u{17B4}'
            | '\u{17B5}'
            | '\u{180E}'
            | '\u{200B}'
            | '\u{200C}'
            | '\u{200D}'
            | '\u{2060}'..='\u{2064}' | '\u{3164}' | '\u{FEFF}' | '\u{FFA0}'
    ) || is_bidi_control(c)
}

/// Invisible characters that have no legitimate use inside a word.
fn is_word_splitter(c: char) -> bool {
    is_invisible(c) && !matches!(c, '\u{200C}' | '\u{200D}') && !is_direction_mark(c)
}

/// Unicode direction controls: marks, embeddings, overrides and isolates.
/// An override can make text read differently from how it is stored
/// (`\u{202E}` turns "gpj.exe" around).
fn is_bidi_control(c: char) -> bool {
    is_direction_mark(c) || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Single direction marks, common in right-to-left text.
fn is_direction_mark(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}')
}

/// `text` without direction controls, for quoting it next to the bot's own
/// words.
pub fn strip_bidi(text: &str) -> Cow<'_, str> {
    if text.chars().any(is_bidi_control) {
        Cow::Owned(text.chars().filter(|c| !is_bidi_control(*c)).collect())
    } else {
        Cow::Borrowed(text)
    }
}

/// `text` without invisible characters (borrowed when there are none).
pub fn strip_invisible(text: &str) -> Cow<'_, str> {
    if text.chars().any(is_invisible) {
        Cow::Owned(text.chars().filter(|c| !is_invisible(*c)).collect())
    } else {
        Cow::Borrowed(text)
    }
}

/// Styled look-alikes of letters and digits: mathematical alphanumerics
/// (`𝐛𝐨𝐥𝐝`, `𝓈𝒸𝓇𝒾𝓅𝓉`), fullwidth ASCII (`ｆｕｌｌ`), enclosed (`Ⓐ①`, `🄰`)
/// and letterlike symbols (`ℂℍℕ`). `№` stays: it is ordinary Russian.
fn is_styled(c: char) -> bool {
    matches!(
        c as u32,
        0x1D400..=0x1D7FF | 0xFF01..=0xFF5E | 0x2460..=0x24FF | 0x1F130..=0x1F189 | 0x2100..=0x2115 | 0x2117..=0x214F
    )
}

/// `text` as the rules should read it: without invisible characters and with
/// styled look-alike letters folded to plain ones (NFKC), so `𝐫𝐞𝐦𝐨𝐭𝐞 𝐰𝐨𝐫𝐤`,
/// `ｔ.ｍｅ/+abc` and `Ⓢⓔⓧ` read as what they say. Other characters are left
/// as they are, so the model sees ordinary text exactly as it was trained.
/// Borrowed when there is nothing to change.
pub fn fold(text: &str) -> Cow<'_, str> {
    let visible = strip_invisible(text);
    if !visible.chars().any(is_styled) {
        return visible;
    }
    let mut folded = String::with_capacity(visible.len());
    for c in visible.chars() {
        if is_styled(c) {
            folded.extend(std::iter::once(c).nfkc());
        } else {
            folded.push(c);
        }
    }
    Cow::Owned(folded)
}

/// Whether an invisible character sits between two letters or digits.
pub fn hides_characters_in_words(text: &str) -> bool {
    let mut before_word_char = false;
    let mut pending_splitter = false;
    for c in text.chars() {
        if is_word_splitter(c) {
            pending_splitter = before_word_char;
            continue;
        }
        let word_char = c.is_alphanumeric();
        if word_char && pending_splitter {
            return true;
        }
        pending_splitter = false;
        before_word_char = word_char;
    }
    false
}

/// Reduce text to a comparable form: lowercase letters and digits only, with
/// runs of the same character collapsed ("Приииивет!!!" == "привет").
/// Emoji-only text falls back to its non-whitespace characters.
pub fn normalize(text: &str) -> String {
    let text = fold(text);
    let alphanumeric: String = text
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    let base = if alphanumeric.is_empty() {
        text.chars().filter(|c| !c.is_whitespace()).collect()
    } else {
        alphanumeric
    };

    let mut collapsed = String::with_capacity(base.len());
    let mut previous = None;
    for c in base.chars() {
        if previous != Some(c) {
            collapsed.push(c);
        }
        previous = Some(c);
    }
    collapsed
}

/// Hash of the normalized text, or `None` when it has fewer than `min_chars`
/// characters (too short to call two messages "the same").
pub fn fingerprint(text: &str, min_chars: usize) -> Option<u64> {
    fingerprint_with_len(text, min_chars).map(|(hash, _)| hash)
}

/// [`fingerprint`] together with the normalized text's length in characters.
pub fn fingerprint_with_len(text: &str, min_chars: usize) -> Option<(u64, usize)> {
    let normalized = normalize(text);
    let chars = normalized.chars().count();
    if chars < min_chars {
        return None;
    }
    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    Some((hasher.finish(), chars))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_ignores_case_punctuation_and_stretched_letters() {
        assert_eq!(normalize("Приииивет, МИР!!!"), normalize("привет мир"));
        assert_eq!(normalize("BUY   now"), "buynow");
        assert_eq!(normalize("😂😂😂"), "😂");
    }

    #[test]
    fn fingerprint_needs_enough_characters() {
        assert_eq!(fingerprint("ок", 3), None);
        assert_eq!(
            fingerprint("Купи СЕЙЧАС!", 3),
            fingerprint("купи сейчас", 3)
        );
    }

    #[test]
    fn strips_zero_width_characters() {
        assert_eq!(strip_invisible("act\u{200B}ive"), "active");
        assert!(matches!(strip_invisible("plain"), Cow::Borrowed("plain")));
    }

    #[test]
    fn folds_styled_letters_to_plain_ones() {
        assert_eq!(fold("𝐫𝐞𝐦𝐨𝐭𝐞 𝐰𝐨𝐫𝐤 𝟑𝟎𝟎$"), "remote work 300$");
        assert_eq!(fold("ｔ．ｍｅ/+abc"), "t.me/+abc");
        assert_eq!(fold("Ⓢⓔⓧⓨ 𝓐𝓷𝓷𝓪"), "Sexy Anna");
        assert_eq!(fold("ин\u{3164}тим"), "интим");
        // Ordinary Russian and English text is left alone, without copying.
        for plain in [
            "Ёлка, йогурт и щи",
            "Hello, world!",
            "Привет 👋❤️",
            "№ 5, 2², ½…",
        ] {
            assert!(matches!(fold(plain), Cow::Borrowed(_)), "{plain}");
        }
    }

    #[test]
    fn styled_copies_have_the_same_fingerprint() {
        assert_eq!(
            fingerprint("𝐂𝐥𝐚𝐢𝐦 𝐲𝐨𝐮𝐫 𝐚𝐢𝐫𝐝𝐫𝐨𝐩", 3),
            fingerprint("Claim your airdrop", 3)
        );
    }

    #[test]
    fn flags_invisible_characters_inside_words() {
        assert!(hides_characters_in_words(
            "Link act\u{200B}ive for 2 mi\u{200B}nutes"
        ));
        assert!(hides_characters_in_words("за\u{2060}работок"));
        assert!(hides_characters_in_words("cli\u{200B}\u{200B}ck"));
        assert!(hides_characters_in_words("pay\u{202E}lap.com"));
        assert!(hides_characters_in_words("pro\u{2066}mo"));
        assert!(hides_characters_in_words("act\u{3164}ive"));
        assert!(hides_characters_in_words("ка\u{2063}зино"));
    }

    #[test]
    fn direction_marks_are_not_spam_but_are_stripped() {
        assert!(!hides_characters_in_words("שלום\u{200F}abc"));
        assert_eq!(strip_bidi("a\u{202E}b\u{200F}c"), "abc");
        assert!(matches!(strip_bidi("plain"), Cow::Borrowed("plain")));
    }

    #[test]
    fn ignores_invisible_characters_outside_words() {
        // Telegram clients put a zero-width space before link previews.
        assert!(!hides_characters_in_words(
            "\u{200B}\u{200B} Выдыхайте парни"
        ));
        assert!(!hides_characters_in_words("конец\u{200B} "));
        // Zero-width joiners build emoji sequences.
        assert!(!hides_characters_in_words("😶\u{200D}🌫️ норм"));
        assert!(!hides_characters_in_words("обычный текст"));
    }
}
