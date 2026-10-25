//! The decision engine: history, trust and learned samples around the
//! classifier.
//!
//! [`Shield::inspect`] is the whole spam decision for one message, minus
//! Telegram I/O, so it can be tested and replayed against real chat
//! exports. The bot binary turns updates into [`Incoming`] messages and
//! carries out the [`Decision`]. Admin feedback (`/spam`, "not spam",
//! `/warn`) flows back in through [`Shield::learn_spam`], [`Shield::vouch`]
//! and [`Shield::warn`].

use std::collections::hash_map::RandomState;
use std::collections::VecDeque;
use std::hash::{BuildHasher, Hasher};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::config::Settings;
use crate::history::{Echoes, History};
use crate::model::Model;
use crate::moderation::{classify, MessageFacts, Reason, Standing, Verdict};
use crate::samples::Samples;
use crate::store::{ReportRecord, SampleKind, Store};
use crate::text;
use crate::trust::Trust;
use crate::warnings::Warnings;

/// Spam reports kept for their buttons.
const MAX_REPORTS: usize = 1000;
/// Accounts posting the same long text before earlier copies are deleted
/// too. With two, either one may be the victim: a troll can repost a
/// newcomer's question to get it deleted, so only the newer copy goes.
const WAVE_SENDERS: usize = 3;

/// One group message, reduced to what the engine needs.
#[derive(Clone, Copy, Debug)]
pub struct Incoming<'a> {
    pub chat: i64,
    /// Telegram user ID, or the chat ID of a channel posting in the group.
    pub sender: i64,
    pub message_id: i32,
    pub facts: MessageFacts<'a>,
    /// The sender is listed in the Combot Anti-Spam database (looked up by
    /// the caller for untrusted senders only).
    pub known_spammer: bool,
    /// An edit of an earlier message: judged like a new one, but it is not
    /// recorded in the history and earns no trust.
    pub edited: bool,
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

/// What a report's buttons carry. IDs repeat (memory mode starts at 1 on
/// every start, a reset database too), so a random token ties the button to
/// the one report it was made for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReportKey {
    pub id: i64,
    pub token: u64,
}

/// Spam reports, in the store or (without one) in memory.
struct Reports {
    store: Option<Store>,
    memory: VecDeque<(ReportKey, ReportRecord)>,
    next_id: i64,
}

/// Model, settings and per-chat memory.
pub struct Shield {
    model: Model,
    settings: Settings,
    trust: Trust,
    history: History,
    samples: Samples,
    warnings: Warnings,
    reports: Reports,
}

impl Shield {
    /// An engine; `store` makes trust, samples, warnings and reports durable
    /// (see ADR 0007).
    pub fn new(model: Model, settings: Settings, store: Option<Store>) -> Shield {
        Shield {
            trust: Trust::new(settings.trust.clone(), store.clone()),
            history: History::new(settings.history_window),
            samples: Samples::new(settings.similarity_threshold, store.clone()),
            warnings: Warnings::new(store.clone()),
            reports: Reports {
                store,
                memory: VecDeque::new(),
                next_id: 1,
            },
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

    /// Number of spam samples learned from admins, in all chats.
    pub fn learned_spam(&self) -> usize {
        self.samples.spam_count()
    }

    /// Decide about `msg` and update history and trust (edits update
    /// neither, except that spam still takes trust away).
    pub fn inspect(&mut self, msg: &Incoming<'_>, now: Instant) -> Decision {
        let text = Some(msg.facts.text).filter(|t| !t.trim().is_empty());
        let echoes = if msg.edited {
            self.history
                .echoes_of_edit(msg.chat, msg.sender, msg.message_id, text, now)
        } else {
            self.history
                .record(msg.chat, msg.sender, msg.message_id, text, now)
        };
        let standing = self.standing(msg.chat, msg.sender, now);
        let vouched = text.is_some_and(|t| self.samples.resembles_ham(msg.chat, t));

        let known_spammer = (msg.known_spammer && !standing.trusted)
            .then(|| spam(Reason::KnownSpammer, Vec::new()));
        let learned = (!standing.trusted
            && !vouched
            && text.is_some_and(|t| self.samples.resembles_spam(msg.chat, t)))
        .then(|| spam(Reason::LearnedSpam, Vec::new()));
        let mut decision = known_spammer
            .or_else(|| self.history_rule(msg.chat, &echoes, standing))
            .or(learned)
            .unwrap_or_else(|| Decision {
                verdict: classify(&self.model, &self.settings, &msg.facts, standing),
                also_delete: Vec::new(),
                newly_trusted: false,
            });
        // An admin said a message like this is fine: judgement calls yield.
        if vouched && matches!(decision.verdict, Verdict::Spam { reason, .. } if reason.is_soft()) {
            decision.verdict = Verdict::Allow;
        }

        if matches!(decision.verdict, Verdict::Spam { .. }) {
            self.trust.revoke(msg.chat, msg.sender);
            return decision;
        }
        if msg.edited {
            return Decision::allow(false);
        }
        let chars = text::strip_invisible(msg.facts.text).trim().chars().count();
        Decision::allow(self.trust.record_clean(msg.chat, msg.sender, chars))
    }

    /// Whether `sender` is trusted in `chat` (trusted members are not looked
    /// up in external spam databases).
    pub fn is_trusted(&self, chat: i64, sender: i64) -> bool {
        self.trust.is_trusted(chat, sender)
    }

    /// An admin marked `text` from `sender` as spam (`/spam`): learn it for
    /// `chat` and withdraw the sender's trust.
    pub fn learn_spam(&mut self, chat: i64, sender: i64, text: &str) {
        self.samples.learn(chat, SampleKind::Spam, text);
        self.trust.revoke(chat, sender);
    }

    /// An admin said `text` from `sender` is not spam: learn it as such for
    /// `chat`, trust the sender and drop their warnings.
    pub fn vouch(&mut self, chat: i64, sender: i64, text: &str) {
        self.samples.learn(chat, SampleKind::Ham, text);
        self.trust.approve(chat, sender);
        self.warnings.clear(chat, sender);
    }

    /// Record an admin warning at `now_unix`. Returns the warning count and
    /// whether the member reached `warn_limit` and should be banned (their
    /// count then starts over).
    pub fn warn(&mut self, chat: i64, sender: i64, now_unix: i64) -> (u32, bool) {
        let count = self.warnings.warn(chat, sender, now_unix);
        let limit = self.settings.warn_limit;
        let ban = limit > 0 && count >= limit;
        if ban {
            self.warnings.clear(chat, sender);
            self.trust.revoke(chat, sender);
        }
        (count, ban)
    }

    /// Keep a spam report for its buttons and return what they carry.
    pub fn save_report(&mut self, report: ReportRecord) -> ReportKey {
        let token = random_token();
        let reports = &mut self.reports;
        if let Some(store) = &reports.store {
            match store.add_report(&report, token, MAX_REPORTS) {
                Ok(id) => return ReportKey { id, token },
                Err(e) => tracing::warn!(error = %e, "report_save_failed"),
            }
        }
        let key = ReportKey {
            id: reports.next_id,
            token,
        };
        reports.next_id += 1;
        if reports.memory.len() >= MAX_REPORTS {
            reports.memory.pop_front();
        }
        reports.memory.push_back((key, report));
        key
    }

    /// The report `key` was made for, if still kept.
    pub fn report(&self, key: ReportKey) -> Option<ReportRecord> {
        let reports = &self.reports;
        if let Some((_, found)) = reports.memory.iter().find(|(k, _)| *k == key) {
            return Some(found.clone());
        }
        let store = reports.store.as_ref()?;
        store.report(key.id, key.token).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "report_load_failed");
            None
        })
    }

    /// Remember that `user` joined `chat` (for the newcomer-link rule).
    pub fn joined(&mut self, chat: i64, user: i64, now: Instant) {
        self.history.joined(chat, user, now);
    }

    /// `sender`'s messages still in the history (to clean up after a ban).
    pub fn recent_from(&self, chat: i64, sender: i64) -> Vec<i32> {
        self.history.recent_from(chat, sender)
    }

    /// Drop expired history, lapsed trust and faded warnings; call
    /// periodically.
    pub fn sweep(&mut self, now: Instant) {
        self.history.sweep(now);
        self.trust.prune();
        self.warnings.prune(crate::trust::unix_now());
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

    /// Repeats apply to everyone. Campaigns apply to untrusted senders and
    /// count only copies from other untrusted senders: a text a regular
    /// posted first is not a spam wave. Earlier copies are deleted only once
    /// [`WAVE_SENDERS`] accounts have posted the text.
    fn history_rule(&self, chat: i64, echoes: &Echoes, standing: Standing) -> Option<Decision> {
        let limit = self.settings.repeat_limit;
        if limit > 0 && echoes.repeats.len() + 1 >= limit {
            return Some(spam(Reason::Repeated, echoes.repeats.clone()));
        }
        if !self.settings.block_campaigns || standing.trusted {
            return None;
        }
        let untrusted: Vec<(i64, i32)> = echoes
            .others
            .iter()
            .copied()
            .filter(|(sender, _)| !self.trust.is_trusted(chat, *sender))
            .collect();
        if untrusted.is_empty() {
            return None;
        }
        let mut senders: Vec<i64> = untrusted.iter().map(|(sender, _)| *sender).collect();
        senders.sort_unstable();
        senders.dedup();
        let wave = senders.len() + 1 >= WAVE_SENDERS;
        let earlier = if wave {
            untrusted.into_iter().map(|(_, id)| id).collect()
        } else {
            Vec::new()
        };
        Some(spam(Reason::Campaign, earlier))
    }
}

/// A non-zero token no earlier run is likely to have handed out. Only
/// uniqueness matters: pressing a button still needs the rights it needs.
fn random_token() -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    hasher.write_u128(nanos);
    hasher.finish().max(1)
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
            known_spammer: false,
            edited: false,
        }
    }

    fn edit(sender: i64, message_id: i32, text: &str) -> Incoming<'_> {
        Incoming {
            edited: true,
            ..msg(sender, message_id, text)
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
        assert!(second.also_delete.is_empty(), "the first may be the victim");
    }

    #[test]
    fn a_third_account_confirms_the_wave_and_clears_it() {
        // Arrange
        let mut shield = shield();
        let start = Instant::now();
        shield.inspect(&msg(1, 1, PROMO), start);
        shield.inspect(&msg(2, 2, PROMO), start + mins(1));

        // Act
        let third = shield.inspect(&msg(3, 3, PROMO), start + mins(2));

        // Assert
        assert_eq!(reason(&third), Some(Reason::Campaign));
        assert_eq!(third.also_delete, vec![1, 2]);
    }

    #[test]
    fn a_repost_does_not_delete_the_original() {
        // Arrange: a newcomer asks a long question; a troll reposts it.
        let mut shield = shield();
        let start = Instant::now();
        let question = "Подскажите пожалуйста, как правильно настроить мост между сетями, \
                        у меня третий день висит транзакция";
        let original = shield.inspect(&msg(1, 10, question), start);

        // Act
        let repost = shield.inspect(&msg(66, 11, question), start + mins(3));

        // Assert
        assert_eq!(original.verdict, Verdict::Allow);
        assert_eq!(reason(&repost), Some(Reason::Campaign));
        assert!(!repost.also_delete.contains(&10));
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
    fn known_spammer_is_blocked_unless_trusted() {
        let mut shield = shield();
        let now = Instant::now();
        let listed = Incoming {
            known_spammer: true,
            ..msg(7, 1, "Добрый вечер, подскажите по мосту")
        };

        assert_eq!(
            reason(&shield.inspect(&listed, now)),
            Some(Reason::KnownSpammer)
        );
        shield.trust.approve(CHAT, 7);
        let again = Incoming {
            message_id: 2,
            ..listed
        };
        assert_eq!(shield.inspect(&again, now).verdict, Verdict::Allow);
    }

    #[test]
    fn one_spam_command_stops_the_reworded_wave() {
        let mut shield = shield();
        let now = Instant::now();
        shield.learn_spam(CHAT, 1, PROMO);

        let reworded =
            "Ondo | доступен клейм $USDC. Команда открыла клейм для активных пользователей";
        assert_eq!(
            reason(&shield.inspect(&msg(2, 1, reworded), now)),
            Some(Reason::LearnedSpam)
        );
    }

    #[test]
    fn a_spam_command_in_one_chat_does_not_reach_another() {
        let mut shield = shield();
        let now = Instant::now();
        let other = Incoming {
            chat: CHAT - 1,
            ..msg(2, 1, PROMO)
        };

        shield.learn_spam(CHAT, 1, PROMO);
        let decision = shield.inspect(&other, now);

        assert_ne!(reason(&decision), Some(Reason::LearnedSpam));
    }

    #[test]
    fn vouched_text_is_no_longer_judged_by_the_model() {
        let mut shield = shield();
        let now = Instant::now();
        let link = "https://docs.arbitrum.foundation/airdrop-eligibility-distribution читайте все";
        assert!(reason(&shield.inspect(&msg(1, 1, link), now)).is_some());

        shield.vouch(CHAT, 1, link);

        assert!(shield.is_trusted(CHAT, 1));
        assert_eq!(
            shield.inspect(&msg(2, 2, link), now).verdict,
            Verdict::Allow
        );
    }

    #[test]
    fn third_warning_means_a_ban() {
        let mut shield = shield();
        let warns: Vec<(u32, bool)> = (0..3)
            .map(|i| shield.warn(CHAT, 1, 1_800_000_000 + i))
            .collect();

        assert_eq!(warns, vec![(1, false), (2, false), (3, true)]);
        assert_eq!(
            shield.warn(CHAT, 1, 1_800_000_010),
            (1, false),
            "count starts over"
        );
    }

    #[test]
    fn reports_are_kept_for_their_buttons() {
        let mut shield = shield();
        let report = ReportRecord {
            chat_id: CHAT,
            sender_id: 1,
            text: "spam".to_string(),
        };

        let key = shield.save_report(report.clone());

        assert_eq!(shield.report(key), Some(report));
        assert_eq!(
            shield.report(ReportKey {
                id: key.id + 1,
                ..key
            }),
            None
        );
    }

    #[test]
    fn a_button_from_before_a_restart_matches_no_newer_report() {
        // Arrange: report 1 before a restart without a store, report 1 after.
        let mut before = shield();
        let old_button = before.save_report(ReportRecord {
            chat_id: CHAT,
            sender_id: 1,
            text: "false positive".to_string(),
        });
        let mut after = shield();
        let new_button = after.save_report(ReportRecord {
            chat_id: CHAT - 1,
            sender_id: 2,
            text: "real spam".to_string(),
        });

        // Act
        let pressed_old = after.report(old_button);

        // Assert
        assert_eq!(old_button.id, new_button.id, "IDs repeat after a restart");
        assert_eq!(pressed_old, None);
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

    #[test]
    fn edit_into_spam_is_caught() {
        let mut shield = shield();
        let now = Instant::now();
        let hello = shield.inspect(&msg(1, 1, "Привет всем, подскажите по мосту"), now);

        let edited = shield.inspect(&edit(1, 1, "вступай t.me/+SecretInvite"), now + mins(5));

        assert_eq!(hello.verdict, Verdict::Allow);
        assert!(reason(&edited).is_some(), "{edited:?}");
    }

    #[test]
    fn edits_are_not_repeats() {
        let mut shield = shield();
        let start = Instant::now();
        let text = "Кто-нибудь знает, когда откроют клейм на мосту?";
        shield.inspect(&msg(1, 1, text), start);

        for i in 1..=3 {
            shield.inspect(&edit(1, 1, text), start + mins(i));
        }
        let second = shield.inspect(&msg(1, 2, text), start + mins(5));

        assert_eq!(second.verdict, Verdict::Allow, "{second:?}");
    }

    #[test]
    fn edit_into_an_earlier_promo_is_a_campaign() {
        let mut shield = shield();
        let start = Instant::now();
        shield.inspect(&msg(1, 1, PROMO), start);
        shield.inspect(&msg(2, 2, "Добрый вечер всем"), start);

        let edited = shield.inspect(&edit(2, 2, PROMO), start + mins(10));

        assert_eq!(reason(&edited), Some(Reason::Campaign));
        assert!(edited.also_delete.is_empty());
    }

    #[test]
    fn edits_earn_no_trust() {
        let mut shield = shield();
        let now = Instant::now();
        shield.inspect(&msg(1, 1, "Первое нормальное сообщение"), now);

        let edits: Vec<bool> = (0..3)
            .map(|_| {
                shield
                    .inspect(&edit(1, 1, "Первое нормальное сообщение, исправил"), now)
                    .newly_trusted
            })
            .collect();

        assert_eq!(edits, vec![false, false, false]);
        assert!(!shield.is_trusted(CHAT, 1));
    }

    #[test]
    fn trusted_members_may_edit_in_a_bare_link() {
        let mut shield = shield();
        let now = Instant::now();
        shield.trust.approve(CHAT, 1);
        let link = "https://docs.arbitrum.foundation/airdrop-eligibility-distribution";

        let edited = shield.inspect(&edit(1, 1, link), now);

        assert_eq!(edited.verdict, Verdict::Allow);
    }
}
