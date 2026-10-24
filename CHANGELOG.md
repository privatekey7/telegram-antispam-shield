# Changelog

All notable changes to this project are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- Messages that reply to a text message from another chat lost their text in
  teloxide and were neither moderated nor read as commands. This includes
  replies to messages from before the group became a supergroup. The bot now
  polls Telegram itself and repairs such updates before parsing them (ADR
  0010). Any text that is still lost is logged as `unreadable_message`.
- `/spam`, `/ban` and `/warn` without a reply, or replying to a message out of
  reach, no longer vanish silently: the bot shows a short hint.

### Security
- The Deploy workflow no longer deploys a pull request from a fork whose
  branch is named `main`. Before, such a pull request passed the
  `branches: [main]` filter: once `DEPLOY_ENABLED` was set, its code was built
  and rolled out to the server that holds `BOT_TOKEN`. Deploys now run only
  for green CI on a push to `main` of the same repository, or a manual run
  from `main`. The commit must be on `origin/main`. Job tokens drop to the
  minimum, and pull request runs can no longer cancel a queued deploy.
  `docs/auto-deploy.md` explains the check and recommends required reviewers
  on the `production` environment.

## [0.2.0] - 2026-10-24

### Added
- Admin commands, as a reply to a message (ADR 0009): `/spam` deletes it and
  the author's recent messages, bans the author and learns the text, so
  similar messages are deleted automatically (cosine of word sets,
  `ANTISPAM_SIMILARITY_THRESHOLD=0.5`: 92% of a real chat's spam matched,
  none of 10,490 normal messages); `/ban`; `/warn` with a ban after
  `ANTISPAM_WARN_LIMIT` (3) warnings in a month.
- Spam reports to `ANTISPAM_REPORT_CHAT` or the owner, with **Not spam**
  (unban/unmute, trust, learn as fine) and **Ban** buttons. Learned samples,
  warnings and reports are stored in `antispam.db`.
- Shape checks for untrusted members, after tg-spam's meta checks, with
  defaults measured on public datasets and a replayed real chat:
  `@somebot`-only triggers (`ANTISPAM_BLOCK_BOT_MENTIONS`, on), words mixing
  Cyrillic and Latin (`ANTISPAM_MIXED_SCRIPT_WORDS=2`), emoji walls
  (`ANTISPAM_EMOJI_LIMIT=10`). Opt-in: `ANTISPAM_BLOCKED_SCRIPTS`,
  `ANTISPAM_STOP_WORDS`, `ANTISPAM_BLOCK_FORWARDS` (forwards, external
  quotes, stories), `ANTISPAM_LINK_LIMIT`, `ANTISPAM_MENTION_LIMIT`.
- CAS lookups (`ANTISPAM_CAS_ENABLED`, on): untrusted members are checked
  against the Combot Anti-Spam database when they post and when they join
  (listed joiners are banned or muted at once if the action allows). Cached
  for an hour; a slow or failing API never blocks the bot.
- `ANTISPAM_DELETE_SERVICE_MESSAGES`: delete "joined" / "left" notices.
- `ANTISPAM_MODEL_MIN_CHARS=20`: shorter messages without links skip the
  model. This removes false positives on "лс" or "всем привет" without
  losing recall on the test data.
- Trusted members (`ANTISPAM_TRUST_AFTER=3`, `ANTISPAM_TRUST_MIN_CHARS=10`,
  ADR 0007): after three clean messages a member skips the model, job-offer
  and newcomer-link checks. Any spam verdict resets them. Trust is saved
  in SQLite (`ANTISPAM_DATA_DIR/antispam.db`; the Docker image uses `/data`
  and `compose.yaml` mounts a named volume there).
- 24-hour chat history (`ANTISPAM_HISTORY_WINDOW_SECS`) with three rules:
  repeats (`ANTISPAM_REPEAT_LIMIT=3`, the same text from one member even
  hours apart), campaigns (`ANTISPAM_BLOCK_CAMPAIGNS`, the same long text
  from several untrusted accounts; all copies are deleted) and newcomer
  links (`ANTISPAM_BLOCK_NEWCOMER_LINKS`, a link from someone who joined
  recently and has not talked yet). Joins are read from service messages
  and `chat_member` updates.
- `ANTISPAM_CLEANUP_ON_BAN` (on): banning or muting a spammer also deletes
  their other messages from the history window.
- Hidden-character rule (`ANTISPAM_BLOCK_HIDDEN_CHARACTERS`, on): an
  invisible character between two letters (`act​ive`) marks spam. The
  classifier now also sees text with invisible characters removed.
- Inline-keyboard rule (`ANTISPAM_BLOCK_INLINE_KEYBOARDS`, on): buttons on a
  member's message, which only automation such as a business bot can attach,
  mark spam. Messages sent via inline bots are not affected.
- Links hidden under text (`text_link` entities) are passed to the rules and
  the model, so a hidden `t.me/+…` invite is caught.
- Job-offer rule (`ANTISPAM_BLOCK_JOB_OFFERS`, on by default): Russian and
  English offers of remote work, side jobs or income are deleted as spam when
  they also mention money or ask to move to private messages. Plain talk about
  work is not affected. On the Telegram spam dataset it flags 43% of spam and
  none of the 14,400 legitimate messages. `/status` shows the setting.
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

- Prebuilt multi-arch images (`linux/amd64`, `linux/arm64`) on GHCR for every
  release (`release.yml`, ADR 0006): tags `X.Y.Z`, `X.Y` and `sha-<commit>`,
  no `latest`. The GitHub Release attaches `compose.yaml` and `env.example`.
- `compose.yaml` in the repository root: run the bot with
  `docker compose up -d`, settings in `.env`, hardened container (read-only,
  no capabilities, 128 MB memory cap, rotated logs).
- `llms-install.md`: a step-by-step install guide for AI agents that checks
  the token, privacy mode, chat ID and admin rights through the Bot API, so
  the user only does the Telegram steps.
- `docs/hosting.md` (EN/RU): Docker Compose, `docker run`, container
  platforms, and a from-source install with a systemd unit.
- Optional auto-deploy from your own fork (`deploy/deploy.sh`,
  `docs/auto-deploy.md`, ADR 0005): the `Deploy` workflow builds the image
  after green CI on `main`, pushes it to GHCR tagged by commit SHA and rolls
  it out over SSH via a forced-command `deploy.sh` with automatic rollback.
  Opt-in via the `DEPLOY_ENABLED` repository variable.
- CI builds the Docker image, validates `compose.yaml`, shellchecks the deploy
  script and lints the workflows (actionlint) on every PR.

### Changed
- The spam decision is now one pure function, `Shield::inspect`
  (`src/shield.rs`). The Telegram glue moved from `main.rs` to `src/bot/`.
- On a replay of a real 13k-message spam-ridden chat, spam caught rose
  from 21% to 99% and false positives fell from 1.8% to 0.18%.
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
- Installation docs are host-neutral: the README quick start uses Docker or an
  AI agent instead of Railway, which is now one platform among many.

### Removed
- "Deploy on Railway" button (Railway no longer has a free tier;
  `railway.json` stays for people who use it).

### Fixed
- Auto-forwarded posts of the group's linked channel and comments made as
  that channel are no longer moderated. Telegram sends them from a non-admin
  service account, so the model could delete the channel's own posts.
- Messages sent on behalf of another channel are moderated. They arrive from
  `@Channel_Bot`, which is marked as a bot, so they used to be skipped.
  Spamming channels are banned with `banChatSenderChat` when the action is
  ban or mute.
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

[Unreleased]: https://github.com/privatekey7/telegram-antispam-shield/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/privatekey7/telegram-antispam-shield/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/privatekey7/telegram-antispam-shield/releases/tag/v0.1.0
