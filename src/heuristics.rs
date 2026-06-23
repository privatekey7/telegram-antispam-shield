//! Fast, high-precision rules that run before the ML model.
//!
//! Heuristics must have a very low false-positive rate: anything ambiguous is
//! left to the model. Today the only hard rule targets Telegram invite links,
//! a near-universal spam signal in public groups.

use regex::Regex;
use std::sync::LazyLock;

/// Telegram private-group invite links (`t.me/+...`, `joinchat`, look-alikes).
static INVITE_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(t\.me/\+|t\.me/joinchat|telegram\.me/joinchat|telegram\.dog/joinchat)")
        .expect("invite-link regex is valid")
});

/// Returns `Some(reason)` when `text` matches an unambiguous spam rule.
pub fn hard_rule(text: &str) -> Option<&'static str> {
    if INVITE_LINK.is_match(text) {
        return Some("telegram_invite_link");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_invite_links() {
        assert_eq!(
            hard_rule("join now t.me/+AbCdEf"),
            Some("telegram_invite_link")
        );
        assert_eq!(
            hard_rule("https://t.me/joinchat/XYZ"),
            Some("telegram_invite_link")
        );
    }

    #[test]
    fn ignores_normal_text_and_public_links() {
        assert_eq!(hard_rule("see our channel t.me/openchannel"), None);
        assert_eq!(hard_rule("привет, как дела?"), None);
    }
}
