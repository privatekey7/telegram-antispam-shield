//! Shape checks: what a message looks like rather than what it says.
//!
//! Inspired by tg-spam's "meta checks". Each was measured on public spam
//! datasets and a replayed real chat before choosing its default (see the
//! README table): mixed-alphabet words, emoji walls and `@somebot`-only
//! messages are on; script blocks, link and mention limits are opt-in.

use std::sync::LazyLock;

use regex::Regex;

/// A message that is nothing but a bot's username (`@gjwgjglkewbot`), the
/// trigger of business-bot spam.
static BOT_MENTION_ONLY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*@[a-z0-9_]{2,}bot\s*$").expect("bot-mention regex is valid")
});

/// `@username` mentions in text.
static MENTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(^|[^\w@])@[A-Za-z][A-Za-z0-9_]{3,}").expect("mention regex is valid")
});

/// Writing systems that can be blocked with `ANTISPAM_BLOCKED_SCRIPTS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    Arabic,
    Armenian,
    Chinese,
    Devanagari,
    Georgian,
    Greek,
    Hebrew,
    Japanese,
    Korean,
    Thai,
}

impl Script {
    /// Parse a script name (case-insensitive; `han` and `cjk` mean Chinese).
    pub fn parse(name: &str) -> Option<Script> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "arabic" => Script::Arabic,
            "armenian" => Script::Armenian,
            "chinese" | "han" | "cjk" => Script::Chinese,
            "devanagari" | "hindi" => Script::Devanagari,
            "georgian" => Script::Georgian,
            "greek" => Script::Greek,
            "hebrew" => Script::Hebrew,
            "japanese" | "kana" => Script::Japanese,
            "korean" | "hangul" => Script::Korean,
            "thai" => Script::Thai,
            _ => return None,
        })
    }

    fn contains(self, c: char) -> bool {
        let c = c as u32;
        match self {
            Script::Arabic => matches!(c, 0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF),
            Script::Armenian => matches!(c, 0x0530..=0x058F),
            Script::Chinese => matches!(c, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF),
            Script::Devanagari => matches!(c, 0x0900..=0x097F),
            Script::Georgian => matches!(c, 0x10A0..=0x10FF),
            Script::Greek => matches!(c, 0x0370..=0x03FF),
            Script::Hebrew => matches!(c, 0x0590..=0x05FF),
            Script::Japanese => matches!(c, 0x3040..=0x30FF),
            Script::Korean => matches!(c, 0xAC00..=0xD7AF | 0x1100..=0x11FF),
            Script::Thai => matches!(c, 0x0E00..=0x0E7F),
        }
    }
}

/// Letters of a blocked script needed before a message counts as written
/// in it (one quoted character is not enough).
pub const BLOCKED_SCRIPT_MIN_LETTERS: usize = 3;

/// The first of `blocked` that `text` is written in, if any.
pub fn blocked_script(text: &str, blocked: &[Script]) -> Option<Script> {
    blocked.iter().copied().find(|script| {
        text.chars().filter(|c| script.contains(*c)).count() >= BLOCKED_SCRIPT_MIN_LETTERS
    })
}

fn is_cyrillic(c: char) -> bool {
    matches!(c as u32, 0x0400..=0x04FF)
}

fn is_latin(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c as u32, 0x00C0..=0x024F) && c.is_alphabetic()
}

/// Words that mix Cyrillic and Latin letters (`зaрaботок` with Latin `a`),
/// the classic homoglyph trick against word filters.
pub fn mixed_script_words(text: &str) -> usize {
    text.split(|c: char| !c.is_alphabetic())
        .filter(|word| word.chars().any(is_cyrillic) && word.chars().any(is_latin))
        .count()
}

/// Emoji in `text` (custom emoji arrive as such placeholders too). A flag is
/// a pair of regional indicator letters and counts once.
pub fn emoji_count(text: &str) -> usize {
    let (mut emoji, mut regional) = (0usize, 0usize);
    for c in text.chars() {
        if is_regional_indicator(c) {
            regional += 1;
        } else if is_emoji(c) {
            emoji += 1;
        }
    }
    emoji + regional.div_ceil(2)
}

fn is_regional_indicator(c: char) -> bool {
    matches!(c as u32, 0x1F1E6..=0x1F1FF)
}

/// Pictographs plus the emoji outside the main blocks that spam walls use:
/// ⭐ ⬇ ⬛ ⏰ ⌛ ▶ ⤴ 〽 ㊙, 🅰 🆓 🈚 🀄 🃏 and the keycap of 1️⃣.
fn is_emoji(c: char) -> bool {
    matches!(
        c as u32,
        0x1F300..=0x1FAFF
            | 0x2600..=0x27BF
            | 0x1F000..=0x1F0FF
            | 0x1F170..=0x1F1E5
            | 0x1F200..=0x1F2FF
            | 0x2B05..=0x2B07
            | 0x2B1B..=0x2B1C
            | 0x2B50
            | 0x2B55
            | 0x231A..=0x231B
            | 0x23E9..=0x23FA
            | 0x25B6
            | 0x25C0
            | 0x25FB..=0x25FE
            | 0x2934..=0x2935
            | 0x3030
            | 0x303D
            | 0x3297
            | 0x3299
            | 0x20E3
    )
}

/// Whether `text` is only a bot's `@username`.
pub fn is_bot_mention_only(text: &str) -> bool {
    BOT_MENTION_ONLY.is_match(text)
}

/// Number of `@username` mentions in `text`.
pub fn mention_count(text: &str) -> usize {
    MENTION.find_iter(text).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_words_mixing_cyrillic_and_latin() {
        // "a" and "o" below are Latin.
        assert_eq!(
            mixed_script_words("Зaрaботок от 500$ в день, пишитe в лc"),
            3
        );
        assert_eq!(mixed_script_words("Привет, как дела с GitHub и Docker?"), 0);
    }

    #[test]
    fn counts_emoji() {
        assert_eq!(emoji_count("👇👇👇👇👇 Best Videos Here🍓"), 6);
        assert_eq!(emoji_count("обычный текст :)"), 0);
        assert_eq!(emoji_count("⭐⭐⭐⭐⭐"), 5);
        assert_eq!(emoji_count("⬇️⬇️ 🆓 1️⃣ ⏰"), 5);
        assert_eq!(emoji_count("🇷🇺🇬🇧🇺🇸"), 3);
        assert_eq!(emoji_count("цена 5€, ©2024, a→b"), 0);
    }

    #[test]
    fn recognises_bot_mention_triggers() {
        assert!(is_bot_mention_only("@gjwgjglkewbot"));
        assert!(is_bot_mention_only(" @SomeHelperBot "));
        assert!(!is_bot_mention_only("@MaxBro25"));
        assert!(!is_bot_mention_only("спроси у @SomeHelperBot"));
    }

    #[test]
    fn finds_blocked_scripts() {
        let spam = "有 至副堡 VX来 帮我收米一天赚几k";
        assert_eq!(
            blocked_script(spam, &[Script::Chinese]),
            Some(Script::Chinese)
        );
        assert_eq!(blocked_script("один иероглиф 中", &[Script::Chinese]), None);
        assert_eq!(blocked_script(spam, &[]), None);
    }

    #[test]
    fn parses_script_names() {
        assert_eq!(Script::parse("Han"), Some(Script::Chinese));
        assert_eq!(Script::parse(" arabic "), Some(Script::Arabic));
        assert_eq!(Script::parse("klingon"), None);
    }

    #[test]
    fn counts_mentions_but_not_emails() {
        assert_eq!(mention_count("пишите @anna_hr или @bob_jobs"), 2);
        assert_eq!(mention_count("mail me at user@example.com"), 0);
    }
}
