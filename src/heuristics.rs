//! Fast, high-precision rules that run before the ML model.
//!
//! Heuristics must have a very low false-positive rate: anything ambiguous is
//! left to the model. Two rules exist today:
//!
//! * Telegram invite links, a near-universal spam signal in public groups.
//! * Job and income offers ("remote work, 300$ a day, DM me"). Real community
//!   chats do not recruit in the open; such offers go to private messages.

use regex::Regex;
use std::sync::LazyLock;

/// Telegram private-group invite links (`t.me/+...`, `joinchat`, look-alikes).
static INVITE_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(t\.me/\+|t\.me/joinchat|telegram\.me/joinchat|telegram\.dog/joinchat)")
        .expect("invite-link regex is valid")
});

/// A web or Telegram link, including spaced-out ones (`https ://`).
static LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(https?\s*:\s*//|\bt\.me/|\btelegram\.me/|\bwww\.)")
        .expect("link regex is valid")
});

/// Wording that offers work or income. On its own this is not enough
/// ("I hate remote work"), so a job offer also needs pay or a contact request.
///
/// Verbs that also mean "it started working" (`заработало`, `заработать`) are
/// left out on purpose: they are everyday words in tech chats.
static JOB_TERMS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)
        удал[её]нн\w*\s+(работ|подработ|заработ|занятост|ваканси|должност)
      | работ\w*\s+(на|из)\s+дом
      | подработ
      | ваканси
      | требу(ется|ются)\s+(сотрудник|работник|менеджер|помощник|люди|курьер|оператор|модератор)
      | (набира[юе]м?|набор)\s+(в\s+команду|сотрудник|работник|людей|персонал)
      | \bзаработ(ок|ка|ку|ком)\b
      | зарабатыва
      | (пассивн|стабильн|дополнительн)\w*\s+доход
      | доход\w*\s+от\s+\d
      | без\s+опыта
      | remote\s+(work|job|position|employment|opportunit)
      | work(ing)?\s+from\s+home
      | part[\s-]?time\s+(job|work|position)
      | side\s+(income|hustle|job)
      | (passive|extra|stable)\s+income
      | (we\s+are|we're|now)\s+hiring
      | job\s+(offer|opportunit)
      | \bvacanc(y|ies)\b
      | no\s+experience\s+(is\s+)?(needed|required|necessary)
      | \bearn(ing)?\s+(money|income|\$|up\s+to|from|\d)
      | make\s+(money|\$\s*\d)
        ",
    )
    .expect("job-terms regex is valid")
});

/// An amount of money: `300$`, `$300`, `5000 руб`, `50к в месяц`, `100 usdt`.
static PAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)
        [$€₽]\s*\d
      | \d[\d\s.,]*\s*([$€₽]|usdt?\b|eur\b|руб|р\.|долл|dollars?\b|bucks\b)
      | \d+\s*(к|k|тыс\w*)\s+(в|за|per|a)\s
        ",
    )
    .expect("pay regex is valid")
});

/// A request to move the conversation to private messages.
static CONTACT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)
        пиш(и|ите)\s+(в\s+)?(лс|л\.с|личк|личн|директ|мне)
      | \b(в|во)\s+(лс|личк\w*|личные)
      | подробност\w*\s+(у|в|по)\s
      | за\s+подробност
      | ставь(те)?\s+\+
      | \b(dm|pm|inbox|message|text|write|contact)\s+(me|us)\b
      | \bin\s+(dm|pm|private)\b
      | \b(dm|pm)\s+for\b
      | send\s+(me\s+)?an?\s+(dm|pm|message)
        ",
    )
    .expect("contact regex is valid")
});

/// Whether `text` contains a Telegram private-group invite link.
pub fn is_invite_link(text: &str) -> bool {
    INVITE_LINK.is_match(text)
}

/// Whether `text` contains a link Telegram might not have marked as one.
pub fn contains_link(text: &str) -> bool {
    LINK.is_match(text)
}

/// Number of links in `text`.
pub fn link_count(text: &str) -> usize {
    LINK.find_iter(text).count()
}

/// Whether `text` offers work or income together with pay or a contact request.
pub fn is_job_offer(text: &str) -> bool {
    JOB_TERMS.is_match(text) && (PAY.is_match(text) || CONTACT.is_match(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_invite_links() {
        assert!(is_invite_link("join now t.me/+AbCdEf"));
        assert!(is_invite_link("https://t.me/joinchat/XYZ"));
    }

    #[test]
    fn ignores_normal_text_and_public_links() {
        assert!(!is_invite_link("see our channel t.me/openchannel"));
        assert!(!is_invite_link("привет, как дела?"));
    }

    #[test]
    fn finds_links_including_spaced_out_ones() {
        assert!(contains_link("https://claim.ondo-foundation.one/"));
        assert!(contains_link("смотри https ://lineascan .build"));
        assert!(contains_link("t.me/somechannel"));
        assert!(!contains_link("встретимся в 10:30, ок?"));
    }

    #[test]
    fn flags_job_offers() {
        let offers = [
            "Looking for people for remote work, 300$ per day, no experience needed, PM me",
            "Нужны люди на удалённую работу, 2-3 часа в день, от 1500 руб. Подробности у @hr_anna",
            "Удаленная работа! Доход от 50к в месяц, пишите в лс",
            "Требуются сотрудники, без опыта, обучение бесплатно. Пиши в личку",
            "Подработка для студентов, 3000р за смену, пишите мне",
            "Стабильный доход без вложений, ставьте + в комментариях",
            "We are hiring! Part-time job, earn $500 a week, DM me",
            "Work from home and earn up to 2000 USDT, message me for details",
            "Passive income opportunity, contact me in DM",
        ];
        for text in offers {
            assert!(is_job_offer(text), "should flag: {text}");
        }
    }

    #[test]
    fn ignores_everyday_talk_about_work() {
        let messages = [
            "У меня заработало, спасибо!",
            "Должно заработать после перезапуска, если нет — пиши в лс",
            "Ищем людей в команду на хакатон, пишите в лс",
            "Я на удалённой работе уже год, очень доволен",
            "Remote work is tiring, I miss the office",
            "What do you think about remote work?",
            "Купил сервер за 5$ в месяц, полёт нормальный",
            "DM me the logs and I will take a look",
            "Earned my first star on GitHub today",
        ];
        for text in messages {
            assert!(!is_job_offer(text), "should allow: {text}");
        }
    }
}
