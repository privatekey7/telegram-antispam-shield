//! Text clean-up shared by the spam rules.
//!
//! Spammers hide words from filters with invisible characters (`act\u{200B}ive`).
//! The classifier sees the cleaned text, and an invisible character between
//! two letters is itself a strong spam signal: people do not type them.

use std::borrow::Cow;

/// Characters that render as nothing. Zero-width (non-)joiners are included
/// for stripping only: they legitimately join emoji and some scripts.
fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}' | '\u{180E}' | '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}'
    )
}

/// Invisible characters that have no legitimate use inside a word.
fn is_word_splitter(c: char) -> bool {
    is_invisible(c) && !matches!(c, '\u{200C}' | '\u{200D}')
}

/// `text` without invisible characters (borrowed when there are none).
pub fn strip_invisible(text: &str) -> Cow<'_, str> {
    if text.chars().any(is_invisible) {
        Cow::Owned(text.chars().filter(|c| !is_invisible(*c)).collect())
    } else {
        Cow::Borrowed(text)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_zero_width_characters() {
        assert_eq!(strip_invisible("act\u{200B}ive"), "active");
        assert!(matches!(strip_invisible("plain"), Cow::Borrowed("plain")));
    }

    #[test]
    fn flags_invisible_characters_inside_words() {
        assert!(hides_characters_in_words(
            "Link act\u{200B}ive for 2 mi\u{200B}nutes"
        ));
        assert!(hides_characters_in_words("за\u{2060}работок"));
        assert!(hides_characters_in_words("cli\u{200B}\u{200B}ck"));
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
