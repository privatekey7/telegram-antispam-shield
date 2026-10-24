//! The decision engine: history and trust around the classifier.
//!
//! [`Shield::inspect`] is the whole spam decision for one message, minus
//! Telegram I/O, so it can be tested and replayed against real chat
//! exports. The bot binary turns updates into [`Incoming`] messages and
//! carries out the [`Decision`].

use std::time::Instant;

use crate::config::Settings;
use crate::history::{Echoes, History};
use crate::model::Model;
use crate::moderation::{classify, MessageFacts, Reason, Standing, Verdict};
use crate::store::Store;
use crate::text;
use crate::trust::Trust;

/// One group message, reduced to what the engine needs.
#[derive(Clone, Copy, Debug)]
pub struct Incoming<'a> {
    pub chat: i64,
    /// Telegram user ID, or the chat ID of a channel posting in the group.
    pub sender: i64,
    pub message_id: i32,
    pub facts: MessageFacts<'a>,
}

/// What to do about one message.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub verdict: Verdict,
    /// Earlier messages to delete together with this one (repeats and
    /// campaign copies from untrusted senders).
    pub also_delete: Vec<i32>,
    /// This message earned the sender trust.
    pub newly_trusted: bool,
}

impl Decision {
    fn allow(newly_trusted: bool) -> Decision {
        Decision {
            verdict: Verdict::Allow,
            also_delete: Vec::new(),
            newly_trusted,
        }
    }
}

/// Model, settings and per-chat memory.
pub struct Shield {
    model: Model,
    settings: Settings,
    trust: Trust,
    history: History,
}

impl Shield {
    /// An engine; `store` makes trust durable (see ADR 0007).
    pub fn new(model: Model, settings: Settings, store: Option<Store>) -> Shield {
        Shield {
            trust: Trust::new(settings.trust.clone(), store),
            history: History::new(settings.history_window),
            model,
            settings,
        }
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Decide about `msg` and update history and trust.
    pub fn inspect(&mut self, msg: &Incoming<'_>, now: Instant) -> Decision {
        let text = Some(msg.facts.text).filter(|t| !t.trim().is_empty());
        let echoes = self
            .history
            .record(msg.chat, msg.sender, msg.message_id, text, now);
        let standing = self.standing(msg.chat, msg.sender, now);

        let decision = self
            .history_rule(msg.chat, &echoes, standing)
            .unwrap_or_else(|| Decision {
                verdict: classify(&self.model, &self.settings, &msg.facts, standing),
                also_delete: Vec::new(),
                newly_trusted: false,
            });

        if matches!(decision.verdict, Verdict::Spam { .. }) {
            self.trust.revoke(msg.chat, msg.sender);
            return decision;
        }
        let chars = text::strip_invisible(msg.facts.text).trim().chars().count();
        Decision::allow(self.trust.record_clean(msg.chat, msg.sender, chars))
    }

    /// Remember that `user` joined `chat` (for the newcomer-link rule).
    pub fn joined(&mut self, chat: i64, user: i64, now: Instant) {
        self.history.joined(chat, user, now);
    }

    /// `sender`'s messages still in the history (to clean up after a ban).
    pub fn recent_from(&self, chat: i64, sender: i64) -> Vec<i32> {
        self.history.recent_from(chat, sender)
    }

    /// Drop expired history; call periodically.
    pub fn sweep(&mut self, now: Instant) {
        self.history.sweep(now);
    }

    fn standing(&self, chat: i64, sender: i64, now: Instant) -> Standing {
        let trusted = self.trust.is_trusted(chat, sender);
        let newcomer = !trusted
            && self.trust.clean_messages(chat, sender) == 0
            && self
                .history
                .joined_within(chat, sender, self.settings.history_window, now);
        Standing { trusted, newcomer }
    }

    /// Repeats apply to everyone; campaigns only to untrusted senders.
    fn history_rule(&self, chat: i64, echoes: &Echoes, standing: Standing) -> Option<Decision> {
        let limit = self.settings.repeat_limit;
        if limit > 0 && echoes.repeats.len() + 1 >= limit {
            return Some(spam(Reason::Repeated, echoes.repeats.clone()));
        }
        if self.settings.block_campaigns && !standing.trusted && !echoes.others.is_empty() {
            let untrusted_copies = echoes
                .others
                .iter()
                .filter(|(sender, _)| !self.trust.is_trusted(chat, *sender))
                .map(|(_, id)| *id)
                .collect();
            return Some(spam(Reason::Campaign, untrusted_copies));
        }
        None
    }
}

fn spam(reason: Reason, also_delete: Vec<i32>) -> Decision {
    Decision {
        verdict: Verdict::Spam { score: 1.0, reason },
        also_delete,
        newly_trusted: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const CHAT: i64 = -100;
    const PROMO: &str =
        "Ondo | апдейт и насыпка $USDC. Сегодня команда открыла клейм для активных пользователей";

    fn shield() -> Shield {
        Shield::new(Model::load_default().unwrap(), Settings::default(), None)
    }

    fn msg(sender: i64, message_id: i32, text: &str) -> Incoming<'_> {
        Incoming {
            chat: CHAT,
            sender,
            message_id,
            facts: MessageFacts::text(text),
        }
    }

    fn reason(decision: &Decision) -> Option<Reason> {
        match decision.verdict {
            Verdict::Spam { reason, .. } => Some(reason),
            Verdict::Allow => None,
        }
    }

    fn mins(n: u64) -> Duration {
        Duration::from_secs(n * 60)
    }

    #[test]
    fn regular_earns_trust_and_may_then_post_bare_links() {
        let mut shield = shield();
        let now = Instant::now();
        let link = "https://docs.arbitrum.foundation/airdrop-eligibility-distribution";

        assert!(reason(&shield.inspect(&msg(1, 1, link), now)).is_some());
        let chat = [
            "Привет всем, давно вас читаю",
            "Кто-нибудь пробовал новый мост?",
            "У меня всё прошло без проблем",
        ];
        let trusted_now: Vec<bool> = chat
            .iter()
            .enumerate()
            .map(|(i, t)| shield.inspect(&msg(1, 10 + i as i32, t), now).newly_trusted)
            .collect();

        assert_eq!(trusted_now, vec![false, false, true]);
        assert_eq!(
            shield.inspect(&msg(1, 20, link), now).verdict,
            Verdict::Allow
        );
    }

    #[test]
    fn third_repeat_within_a_day_deletes_all_copies() {
        let mut shield = shield();
        let start = Instant::now();
        let spam_text = "有 至副堡 VX来 帮我收米一天赚几k @lpbxya";

        shield.inspect(&msg(1, 1, spam_text), start);
        shield.inspect(&msg(1, 2, spam_text), start + mins(22));
        let third = shield.inspect(&msg(1, 3, spam_text), start + mins(44));

        assert_eq!(reason(&third), Some(Reason::Repeated));
        assert_eq!(third.also_delete, vec![1, 2]);
    }

    #[test]
    fn same_promo_from_a_second_account_is_a_campaign() {
        let mut shield = shield();
        let start = Instant::now();

        shield.inspect(&msg(1, 1, PROMO), start);
        let second = shield.inspect(&msg(2, 2, PROMO), start + mins(80));

        assert_eq!(reason(&second), Some(Reason::Campaign));
        assert_eq!(second.also_delete, vec![1]);
    }

    #[test]
    fn newcomer_link_is_blocked_until_they_talk() {
        let mut shield = shield();
        let now = Instant::now();
        let link = Incoming {
            facts: MessageFacts {
                text: "Посмотрите мой проект",
                has_link: true,
                ..MessageFacts::default()
            },
            ..msg(5, 1, "")
        };
        shield.joined(CHAT, 5, now);

        assert_eq!(
            reason(&shield.inspect(&link, now)),
            Some(Reason::NewcomerLink)
        );
        let hello = shield.inspect(&msg(5, 2, "Подскажите, где почитать правила чата?"), now);
        assert_eq!(hello.verdict, Verdict::Allow, "{hello:?}");
        assert_ne!(
            reason(&shield.inspect(
                &Incoming {
                    message_id: 3,
                    ..link
                },
                now
            )),
            Some(Reason::NewcomerLink)
        );
    }

    #[test]
    fn spam_takes_trust_away() {
        let mut shield = shield();
        let now = Instant::now();
        for (i, t) in [
            "Первое нормальное сообщение",
            "Второе нормальное сообщение",
            "Третье нормальное сообщение",
        ]
        .iter()
        .enumerate()
        {
            shield.inspect(&msg(1, i as i32, t), now);
        }

        shield.inspect(&msg(1, 9, "вступай t.me/+SecretInvite"), now);

        assert!(!shield.standing(CHAT, 1, now).trusted);
    }
}
