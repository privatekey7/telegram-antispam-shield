//! Train/serve parity: the Rust scorer must reproduce the Python trainer's
//! probabilities. Expected values come from `scripts/train_model.py` running on
//! the exact model shipped in `model/spam_model.bin`.

use telegram_antispam_shield::model::Model;

/// (message, expected probability from the Python reference).
const FIXTURES: &[(&str, f32)] = &[
    ("Привет, как дела? Когда встреча по проекту?", 0.253570),
    (
        "ЗАРАБОТОК ОТ 5000$ В ДЕНЬ! Пиши в личку @easymoney переходи t.me/+abc",
        1.000000,
    ),
    ("Thanks for the help, the build passes now.", 0.235586),
    (
        "FREE crypto airdrop!!! Click http://bit.ly/xx claim now WINNER",
        0.999999,
    ),
    (
        "Ищу людей в команду, удалённо, доход до 150000р, пишите +",
        0.999998,
    ),
    (
        "Документацию можно посмотреть в разделе настроек профиля",
        0.000028,
    ),
];

#[test]
fn rust_scorer_matches_python_reference() {
    let model = Model::load_default().expect("embedded model loads");
    for (text, expected) in FIXTURES {
        let got = model.score(text);
        let diff = (got - expected).abs();
        assert!(
            diff < 0.01,
            "score mismatch for {text:?}: got {got:.6}, expected {expected:.6} (diff {diff:.6})"
        );
    }
}

#[test]
fn classification_matches_threshold() {
    let model = Model::load_default().expect("embedded model loads");
    for (text, expected) in FIXTURES {
        let (is_spam, _) = model.is_spam(text, None);
        assert_eq!(
            is_spam,
            *expected >= model.threshold,
            "classification mismatch for {text:?}"
        );
    }
}
