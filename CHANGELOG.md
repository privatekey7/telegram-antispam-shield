# Changelog

All notable changes to this project are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Chat allowlist via `ANTISPAM_ALLOWED_CHATS`: when set, the bot only moderates
  the listed chat IDs and ignores every other chat. Protects public bots from
  strangers adding them and consuming the owner's hosting quota.
- `ANTISPAM_LEAVE_UNKNOWN_CHATS`: optionally make the bot leave any chat that is
  not on the allowlist instead of silently ignoring it.
- `ANTISPAM_OWNER_ID`: restrict the private-chat `/status` command to the
  configured owner so bot configuration is not exposed to strangers.

### Changed
- `/status` now also reports `allowed_chats` and `leave_unknown_chats`.

## [0.1.0] - 2026-06-23

### Added
- Telegram anti-spam bot in Rust using `teloxide` (long polling).
- Embedded linear ML classifier (hashed char/word n-grams) trained on public
  Russian + English spam datasets. ROC-AUC ≈ 0.99, false-positive rate ≈ 1%.
- High-precision heuristic prefilter for Telegram invite links.
- Configurable actions: delete only / delete + ban / delete + mute.
- Dry-run mode, admin exemption with a TTL cache, threshold override.
- Train/serve parity tests between the Python trainer and the Rust scorer.
- Multi-stage, non-root Dockerfile and Railway deploy config.
- GitHub Actions CI: rustfmt, clippy, tests, `cargo audit`, gitleaks.

[0.1.0]: https://github.com/zalupajap/telegram-antispam-shield/releases/tag/v0.1.0
