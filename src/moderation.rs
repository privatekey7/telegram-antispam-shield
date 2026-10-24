//! Combines heuristics and the ML model into a single decision.

use std::borrow::Cow;

use crate::config::Settings;
use crate::heuristics;
use crate::model::Model;
use crate::text;

/// What the classifier knows about one message.
#[derive(Clone, Copy, Debug, Default)]
pub struct MessageFacts<'a> {
    /// Text or caption as the reader sees it.
    pub text: &'a str,
    /// Targets of links hidden under words (`text_link` entities).
    pub hidden_links: &'a [String],
    /// Inline buttons on a message from a person rather than an inline bot.
    /// Only automation (for example a business bot) can attach them.
    pub inline_keyboard: bool,
}

impl<'a> MessageFacts<'a> {
    /// Facts for a plain text message.
    pub fn text(text: &'a str) -> Self {
        MessageFacts {
            text,
            ..MessageFacts::default()
        }
    }
}

/// Which rule marked a message as spam.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    InlineKeyboard,
    HiddenCharacters,
    InviteLink,
    JobOffer,
    Model,
}

impl Reason {
    /// Stable name for logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::InlineKeyboard => "inline_keyboard",
            Reason::HiddenCharacters => "hidden_characters",
            Reason::InviteLink => "telegram_invite_link",
            Reason::JobOffer => "job_offer",
            Reason::Model => "ml_model",
        }
    }
}

/// The outcome of classifying one message.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Message is allowed; do nothing.
    Allow,
    /// Message is spam. `score` is the confidence; `reason` is the trigger.
    Spam { score: f32, reason: Reason },
}

/// Classify a message using the configured heuristics and model.
///
/// Heuristics run first (fast, high precision). If none fire, the ML model
/// decides using the effective threshold. Rules and model see the text
/// without invisible characters and with hidden link targets appended.
pub fn classify(model: &Model, settings: &Settings, facts: &MessageFacts<'_>) -> Verdict {
    let rule = |reason| Verdict::Spam { score: 1.0, reason };

    if settings.block_inline_keyboards && facts.inline_keyboard {
        return rule(Reason::InlineKeyboard);
    }
    if settings.block_hidden_characters && text::hides_characters_in_words(facts.text) {
        return rule(Reason::HiddenCharacters);
    }

    let content = classifier_input(facts);
    if content.trim().is_empty() {
        return Verdict::Allow;
    }
    if settings.block_invite_links && heuristics::is_invite_link(&content) {
        return rule(Reason::InviteLink);
    }
    if settings.block_job_offers && heuristics::is_job_offer(&content) {
        return rule(Reason::JobOffer);
    }

    let (is_spam, score) = model.is_spam(&content, settings.threshold);
    if is_spam {
        Verdict::Spam {
            score,
            reason: Reason::Model,
        }
    } else {
        Verdict::Allow
    }
}

/// Visible text without invisible characters, followed by hidden link targets.
fn classifier_input<'a>(facts: &MessageFacts<'a>) -> Cow<'a, str> {
    let visible = text::strip_invisible(facts.text);
    if facts.hidden_links.is_empty() {
        return visible;
    }
    let mut content = visible.into_owned();
    for link in facts.hidden_links {
        content.push(' ');
        content.push_str(link);
    }
    Cow::Owned(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(facts: &MessageFacts<'_>) -> Verdict {
        let model = Model::load_default().unwrap();
        classify(&model, &Settings::default(), facts)
    }

    fn reason_of(verdict: Verdict) -> Option<Reason> {
        match verdict {
            Verdict::Spam { reason, .. } => Some(reason),
            Verdict::Allow => None,
        }
    }

    #[test]
    fn obvious_spam_is_flagged() {
        let spam = "ЗАРАБОТОК ОТ 5000$ В ДЕНЬ! Пиши в личку @easymoney, доход гарантирован";
        assert!(matches!(
            check(&MessageFacts::text(spam)),
            Verdict::Spam { .. }
        ));
    }

    #[test]
    fn normal_message_is_allowed() {
        let ham = "Привет! Когда у нас созвон по проекту завтра?";
        assert_eq!(check(&MessageFacts::text(ham)), Verdict::Allow);
    }

    #[test]
    fn invite_link_is_flagged_by_heuristic() {
        let verdict = check(&MessageFacts::text("join t.me/+SecretInvite"));
        assert_eq!(reason_of(verdict), Some(Reason::InviteLink));
    }

    #[test]
    fn job_offer_is_flagged_by_heuristic() {
        let offer = "Looking for people for remote work, 300$ per day, PM me";
        assert_eq!(
            reason_of(check(&MessageFacts::text(offer))),
            Some(Reason::JobOffer)
        );
    }

    #[test]
    fn job_offer_rule_can_be_disabled() {
        let model = Model::load_default().unwrap();
        let settings = Settings {
            block_job_offers: false,
            ..Settings::default()
        };
        let offer = "Удалённая работа для всех, пишите в лс";
        let verdict = classify(&model, &settings, &MessageFacts::text(offer));
        assert_ne!(reason_of(verdict), Some(Reason::JobOffer));
    }

    #[test]
    fn buttons_from_a_person_are_spam() {
        let facts = MessageFacts {
            text: "😉🍯 🙀🎵 🌺😁",
            inline_keyboard: true,
            ..MessageFacts::default()
        };
        assert_eq!(reason_of(check(&facts)), Some(Reason::InlineKeyboard));
    }

    #[test]
    fn buttons_without_text_are_spam() {
        let facts = MessageFacts {
            inline_keyboard: true,
            ..MessageFacts::default()
        };
        assert_eq!(reason_of(check(&facts)), Some(Reason::InlineKeyboard));
    }

    #[test]
    fn invisible_characters_inside_words_are_spam() {
        let text = "I’m tired of wearing th\u{200B}is dress... Tap below to see\u{200B} more";
        assert_eq!(
            reason_of(check(&MessageFacts::text(text))),
            Some(Reason::HiddenCharacters)
        );
    }

    #[test]
    fn hidden_invite_link_is_found() {
        let links = ["https://t.me/+SecretInvite".to_string()];
        let facts = MessageFacts {
            text: "Заходи к нам, тут интересно",
            hidden_links: &links,
            ..MessageFacts::default()
        };
        assert_eq!(reason_of(check(&facts)), Some(Reason::InviteLink));
    }

    #[test]
    fn empty_message_is_allowed() {
        assert_eq!(check(&MessageFacts::text(" \u{200B} ")), Verdict::Allow);
    }
}
