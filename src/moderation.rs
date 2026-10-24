//! Combines heuristics and the ML model into a single decision.

use crate::config::Settings;
use crate::heuristics;
use crate::model::Model;

/// The outcome of classifying one message.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Message is allowed; do nothing.
    Allow,
    /// Message is spam. `score` is the confidence; `reason` is the trigger.
    Spam { score: f32, reason: &'static str },
}

/// Classify `text` using the configured heuristics and model.
///
/// Heuristics run first (fast, high precision). If none fire, the ML model
/// decides using the effective threshold.
pub fn classify(model: &Model, settings: &Settings, text: &str) -> Verdict {
    if settings.block_invite_links && heuristics::is_invite_link(text) {
        return Verdict::Spam {
            score: 1.0,
            reason: "telegram_invite_link",
        };
    }
    if settings.block_job_offers && heuristics::is_job_offer(text) {
        return Verdict::Spam {
            score: 1.0,
            reason: "job_offer",
        };
    }

    let (is_spam, score) = model.is_spam(text, settings.threshold);
    if is_spam {
        Verdict::Spam {
            score,
            reason: "ml_model",
        }
    } else {
        Verdict::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Action;

    fn settings() -> Settings {
        Settings {
            action: Action::DeleteOnly,
            dry_run: false,
            threshold: None,
            block_invite_links: true,
            block_job_offers: true,
            exempt_admins: true,
            admin_cache_ttl_secs: 3600,
            allowed_chats: None,
            leave_unknown_chats: false,
            owner_id: None,
            flood: Default::default(),
        }
    }

    #[test]
    fn obvious_spam_is_flagged() {
        let model = Model::load_default().unwrap();
        let s = settings();
        let spam = "ЗАРАБОТОК ОТ 5000$ В ДЕНЬ! Пиши в личку @easymoney, доход гарантирован";
        assert!(matches!(classify(&model, &s, spam), Verdict::Spam { .. }));
    }

    #[test]
    fn normal_message_is_allowed() {
        let model = Model::load_default().unwrap();
        let s = settings();
        let ham = "Привет! Когда у нас созвон по проекту завтра?";
        assert_eq!(classify(&model, &s, ham), Verdict::Allow);
    }

    #[test]
    fn invite_link_is_flagged_by_heuristic() {
        let model = Model::load_default().unwrap();
        let s = settings();
        match classify(&model, &s, "join t.me/+SecretInvite") {
            Verdict::Spam { reason, .. } => assert_eq!(reason, "telegram_invite_link"),
            Verdict::Allow => panic!("invite link should be spam"),
        }
    }

    #[test]
    fn job_offer_is_flagged_by_heuristic() {
        let model = Model::load_default().unwrap();
        let offer = "Looking for people for remote work, 300$ per day, PM me";
        match classify(&model, &settings(), offer) {
            Verdict::Spam { reason, .. } => assert_eq!(reason, "job_offer"),
            Verdict::Allow => panic!("job offer should be spam"),
        }
    }

    #[test]
    fn job_offer_rule_can_be_disabled() {
        let model = Model::load_default().unwrap();
        let s = Settings {
            block_job_offers: false,
            ..settings()
        };
        let offer = "Удалённая работа для всех, пишите в лс";
        assert!(!matches!(
            classify(&model, &s, offer),
            Verdict::Spam {
                reason: "job_offer",
                ..
            }
        ));
    }
}
