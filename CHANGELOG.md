# Changelog

All notable changes to this project are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-10-25

### Upgrade notes
- **The bot now moderates only the chats in `ANTISPAM_ALLOWED_CHATS`.**
  Without the list it moderates no chat and logs `chat_not_allowed` with the
  ID of each group it sees. Add your group IDs before upgrading, or set
  `ANTISPAM_ALLOW_ALL_CHATS=true` to keep the old behaviour (not
  recommended). See ADR 0012.
- `/spam`, `/ban`, `/warn` and the report's **Ban** button now need the
  **Delete messages** right as well as **Ban users**.
- The database schema moves to version 4 (report tokens). Back up
  `antispam.db` before upgrading; v0.2.x cannot use report buttons made by
  v0.3. Report buttons from v0.2.x keep working.
- Auto-deploy only: the Deploy workflow sends
  `deploy-<commit>@sha256:<digest>` to the server. Copy the new
  `deploy/deploy.sh` to the server first (docs/auto-deploy.md).
- compose.yaml defaults to the `0.3` image line: download the new
  compose.yaml to upgrade.

### Changed
- Trust lapses after 90 days without a clean message
  (`ANTISPAM_TRUST_TTL_DAYS`, `0` = never); the member then earns it again.
- The campaign rule deletes earlier copies of a text only once three
  accounts have posted it; with two, only the newer copy is deleted.
- The runtime image is distroless (no shell, no package manager) and still
  runs as UID 10001. compose.yaml adds `pids_limit: 128`.

### Fixed
- Poll questions and options, checklists, venues and contacts are now
  checked like text. Before, spam in them was never seen.
- In a forum topic, `/ban` or `/spam` without a reply no longer targets the
  topic's creation notice (and so the topic's creator); the bot shows the
  usual hint.
- Docs: the `docker run` example mounts the tmpfs the health check needs and
  sets the same limits as compose.yaml; signing is documented from 0.2.1
  on; `ANTISPAM_CAS_URL` is documented; the data volume is recommended; the
  0.2.1 notes now give the real retry cap (5 s).

### Security
- Report buttons carry a random token next to the report ID. Before, IDs
  started over after a restart without the data volume or a database
  reset, and an old **Not spam** or **Ban** button acted on whichever new
  report had its number, possibly in another chat.
- One busy chat can no longer stall every other chat. The poller drops an
  update of a chat whose queue (now 256) is full, or when 4,096 updates
  wait in total, instead of blocking the dispatcher; drops are logged
  (`chat_backlog_full`, `chat_backlog_recovered`).
- A failed CAS lookup pauses CAS for a minute, so an outage costs one timeout
  instead of up to 5 s per message, and CAS answers over 64 KB are refused.
  A CAS URL that is not https gets a startup warning.
- Actions that delete messages need the **Delete messages** right: an admin
  with only **Ban users** could make the bot delete messages, and through
  `/spam` teach it to delete other members' messages.
- A repost of someone's long message no longer gets the original deleted
  through the campaign rule.
- Direction overrides and isolates (U+202A–U+202E, U+2066–U+2069) inside a
  word count as hidden characters. Reports split quoted text on every line
  break a client may render (U+2028, U+2029, U+0085, …) and drop direction
  controls, so spam cannot fake lines of the report.
- Members silent for a month (untrusted) or past their trust (trusted) and
  warnings that no longer count are deleted from the database.
- Releases are published only for a tag whose commit is on `main` and passed
  CI. GitHub Actions are pinned to commit SHAs. Auto-deploy images are
  tagged `deploy-<commit>` (no longer clashing with the release's
  `sha-<commit>`), signed with cosign and pulled by digest.

## [0.2.2] - 2026-10-25

### Fixed
- `/spam`, `/ban` and `/warn` sent as a reply to an admin, the bot's owner or
  a post of the group itself (an anonymous admin, the linked channel) no
  longer act. Before, `/warn` counted warnings against an admin, and the bans
  failed in Telegram. The bot now shows a 20-second hint instead.

## [0.2.1] - 2026-10-25

### Upgrade notes
- The database schema moves to version 3 (per-chat samples). v0.2.0 cannot
  run on it: back up `antispam.db` before upgrading, and restore that copy
  to roll back (ADR 0011). Samples learned before the upgrade are set aside
  and no longer applied.
- Startup now fails if `ANTISPAM_ALLOWED_CHATS` or `ANTISPAM_OWNER_ID`
  contains an entry that is not a number.

### Fixed
- Messages that reply to a text message from another chat lost their text in
  teloxide and were neither moderated nor read as commands. This includes
  replies to messages from before the group became a supergroup. The bot now
  polls Telegram itself and repairs such updates before parsing them (ADR
  0010). Any text that is still lost is logged as `unreadable_message`.
- `/spam`, `/ban` and `/warn` without a reply, or replying to a message out of
  reach, no longer vanish silently: the bot shows a short hint.

### Security
- Learned samples (`/spam`, **Not spam**, **Ban** in a report) now apply only
  in the chat where they were taught. Before, an admin of any chat with the
  bot could make it delete messages and ban people in other chats, and push
  out the samples other chats had learned (ADR 0011).
- Learned samples are bounded: only the first 800 chars and 128 distinct words
  of a text are kept, a text with the same words is stored once, and each chat
  keeps at most 1,000 samples per kind (5,000 in total, taken first from the
  chat holding the most). Before, repeated `/spam` on one long message could
  exhaust the container's memory and crash the bot on every restart.
- Upgrading sets the old shared samples aside (table `samples_unscoped`, never
  loaded): every chat starts with no learned samples. ADR 0011 shows how to
  copy them into a chat by hand.
- Edited messages are now moderated. Before, a member could post a harmless
  text and later edit it into spam unseen, since the bot did not even
  subscribe to `edited_message` updates. An edit is checked like a new
  message, and a report marks it as edited. It does not count toward
  anti-flood, repeats or trust, and cannot run admin commands.
- The Deploy workflow no longer deploys a pull request from a fork whose
  branch is named `main`. Before, such a pull request passed the
  `branches: [main]` filter: once `DEPLOY_ENABLED` was set, its code was built
  and rolled out to the server that holds `BOT_TOKEN`. Deploys now run only
  for green CI on a push to `main` of the same repository, or a manual run
  from `main`. The commit must be on `origin/main`. Job tokens drop to the
  minimum, and pull request runs can no longer cancel a queued deploy.
  `docs/auto-deploy.md` explains the check and recommends required reviewers
  on the `production` environment.
- `deploy.sh` checks the whole image tag instead of line by line: a tag with
  a newline (`sha-<sha>\nIMAGE=...`) passed the check and wrote extra lines
  into `.env`, which got around the SSH forced command. A failed pull or
  start now rolls back instead of leaving `.env` on the new tag.
  `deploy/test-deploy.sh` covers both.
- Admin commands and report buttons now check the admin's rights (ADR 0003
  addendum). `/spam`, `/ban`, the **Ban** button and **Not spam** need the
  **Ban users** right (`can_restrict_members`); `/warn` needs it too while
  warnings can end in a ban, otherwise **Delete messages**. Before, any admin,
  even one with only a title, could make the bot ban.
- Anonymous admins (writing as the group) can no longer use commands, because
  their rights cannot be checked. They get a hint instead.
- The bot confirms an admin's rights with Telegram before acting, so a
  demoted admin loses access at once instead of after the admin cache TTL.
- `ANTISPAM_ALLOWED_CHATS` or `ANTISPAM_OWNER_ID` with an entry that is not a
  number now stops the bot at startup. Before, a typo such as `@mygroup`
  silently allowed every chat.
- Without `ANTISPAM_OWNER_ID`, `/status` no longer shows the configuration to
  anyone; it only says that the bot is running.
- Each chat sends at most 20 spam reports per 10 minutes, so one chat cannot
  flood the owner or push other reports out of storage.
- The bot warns at startup when `ANTISPAM_ALLOWED_CHATS` or
  `ANTISPAM_OWNER_ID` is unset, and the docs now strongly recommend the
  allowlist.
- `ANTISPAM_BLOCK_FORWARDS` again catches replies that quote a text message
  from another chat: the poller remembers which messages lost such a quote
  during repair.
- Admin checks no longer hold the shared chat cache lock during Bot API
  calls, and a failed admin lookup is not retried for 60 s.
- Group notices (`/warn`, flood mutes) no longer repeat names verbatim:
  words that look like links or mentions are dropped and the name is cut to
  32 characters (`id N` if nothing is left), so ads in names are not
  amplified. Reports mark everything from the reported chat (title, name in
  «», each line of the text behind `│`) and never show link previews.
- Deletes, bans and mutes are retried once after Telegram's `retry_after`
  (up to 5 s) instead of being lost during a raid.
- The image has a `HEALTHCHECK` (`telegram-antispam-shield healthcheck`, fed
  by a heartbeat after each `getUpdates`); compose.yaml mounts a tmpfs on
  `/tmp` for it.
- Release images carry an SPDX SBOM and SLSA provenance and are signed with
  keyless cosign; base images are pinned by digest. Verification steps are in
  docs/hosting.md.
- CI checks the SHA-256 of the gitleaks and actionlint downloads instead of
  running a remote install script, and declares `permissions: contents: read`.
- The CAS cache can no longer grow past 10,000 entries.
- Docs no longer put the bot token in browser URLs or curl arguments, no
  longer suggest sending it to the assistant, and use `restrict` in the deploy
  key's `authorized_keys` line.
- Once the total sample budget (5,000 per kind) is full, the chat that adds
  a sample drops its own oldest, so other chats can no longer shrink a
  chat's samples. Only a chat's first sample is taken from the largest chat.
- Every `getUpdates` carries the allowed-updates list. Before, a failed
  first call after an upgrade left edited messages unmoderated until the
  next restart.
- After a 429 only one handler waits, for at most 5 s; other calls give up
  at once, so a raid in one chat cannot stall moderation in all of them.
- Each schema migration commits together with its version, so a crash
  midway no longer leaves a half-built database.

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

[Unreleased]: https://github.com/privatekey7/telegram-antispam-shield/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/privatekey7/telegram-antispam-shield/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/privatekey7/telegram-antispam-shield/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/privatekey7/telegram-antispam-shield/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/privatekey7/telegram-antispam-shield/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/privatekey7/telegram-antispam-shield/releases/tag/v0.1.0
