# Changelog

All notable changes to this project are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Anti-flood (`src/flood.rs`): per-user sliding windows catch users who send
  more than `ANTISPAM_FLOOD_MAX_MESSAGES` messages in
  `ANTISPAM_FLOOD_WINDOW_SECS` (albums count once; stickers and media count) or
  repeat the same normalized text `ANTISPAM_FLOOD_MAX_DUPLICATES` times within
  `ANTISPAM_FLOOD_DUPLICATE_WINDOW_SECS`. The whole burst is deleted and the
  user is muted with escalating steps (`ANTISPAM_FLOOD_MUTE_STEPS`, default
  10m → 1h → 1d, reset after `ANTISPAM_FLOOD_STRIKE_RESET_SECS`). An optional
  self-deleting notice tells the chat what happened. On by default; respects
  dry-run and admin exemption.
- Chat allowlist via `ANTISPAM_ALLOWED_CHATS`: when set, the bot only moderates
  the listed chat IDs and ignores every other chat. Protects public bots from
  strangers adding them and consuming the owner's hosting quota.
- `ANTISPAM_LEAVE_UNKNOWN_CHATS`: optionally make the bot leave any chat that is
  not on the allowlist instead of silently ignoring it.
- `ANTISPAM_OWNER_ID`: restrict the private-chat `/status` command to the
  configured owner so bot configuration is not exposed to strangers.

- Self-hosted VPS deploy (`deploy/`, `docs/deploy-vps.md`, ADR 0005): the
  `Deploy` workflow builds the image after green CI on `main`, pushes it to
  GHCR tagged by commit SHA and rolls it out over SSH via a forced-command
  `deploy.sh` with automatic rollback. Opt-in via the `DEPLOY_ENABLED`
  repository variable.
- CI builds the Docker image, validates the Compose file and shellchecks the
  deploy script on every PR.

### Changed
- `/status` now also reports `allowed_chats`, `leave_unknown_chats` and the
  anti-flood limits.
- Admin exemption is now checked for every message (including media), since
  anti-flood looks at all message types; the admin list stays cached.
- Dependencies refreshed within the declared MSRV (Rust 1.88); `cargo update`
  now stays MSRV-compatible via `.cargo/config.toml`.
- Docker images moved to Debian 13 (trixie): `rust:1.99-slim-trixie` builder,
  `debian:trixie-slim` runtime.
- Dependabot keeps cargo crates, GitHub Actions and Docker images up to date.
- Repository links point to `privatekey7/telegram-antispam-shield`.

### Fixed
- The bot now compiles on non-Unix hosts (local development on Windows); the
  SIGTERM handler is Unix-only.

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

[0.1.0]: https://github.com/privatekey7/telegram-antispam-shield/releases/tag/v0.1.0
