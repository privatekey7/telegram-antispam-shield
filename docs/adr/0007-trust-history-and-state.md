# 7. Trusted members, chat history and an optional state file

- Status: accepted
- Date: 2026-10-24

## Context

We replayed the full history of a real, spam-ridden group (13,221 messages,
459 members, 2022–2026) through the bot. Two problems showed up that a
per-message classifier cannot solve:

1. **False positives on regulars.** The model calls almost any bare link spam.
   117 messages from long-time members (links to GitHub, YouTube, docs, block
   explorers) would have been deleted, about 1.8% of their messages.
2. **Spam that only looks like spam in context.**
   - A bot posted the same Chinese text 15 times, every 10–20 minutes. The
     anti-flood window (2 min) never saw three copies.
   - About 25 fresh accounts each posted one phishing "claim your airdrop"
     message with a link, often the same text. Each account posted once. To
     the model the text looked like the chat's normal crypto talk (score 0.0).

tg-spam, the most widely used open-source Telegram anti-spam bot, solves (1)
with "approved users": content checks stop after a user's first clean
message. It solves part of (2) with a duplicate detector. Approved users live
in SQLite.

## Decision

- **Trust (`trust.rs`).** A member is trusted in a chat after
  `ANTISPAM_TRUST_AFTER` (default 3) clean messages of at least
  `ANTISPAM_TRUST_MIN_CHARS` (default 10) characters. Short messages do not
  count, so trust cannot be farmed with "+" or "hi".
  - Trusted members skip the content checks: the model, job offers and
    newcomer links.
  - Rules with no false positives in the replay still apply to everyone:
    inline buttons, invisible characters, invite links and repeats.
  - Any spam verdict resets the member to zero.
  - `0` turns trust off and checks everyone, like tg-spam's "paranoid" mode.
  - We chose 3 messages over tg-spam's 1. In the replay spam recall was the
    same for 1 to 5. Three messages of at least 10 characters cost one extra
    false positive (18 vs 17) and make warming up an account slower.
- **History (`history.rs`).** Messages and joins are kept in memory for
  `ANTISPAM_HISTORY_WINDOW_SECS` (default 24 h, at most 5,000 messages per
  chat). The history feeds three rules and one clean-up:
  - repeats: the same normalized text from one sender `ANTISPAM_REPEAT_LIMIT`
    (3) times. Texts must have at least 15 characters. This applies to
    everyone, like tg-spam's duplicates;
  - campaigns: the same text of at least 40 characters from other accounts.
    This applies to untrusted senders. Earlier copies from untrusted senders
    are deleted too;
  - newcomer links: a link from someone who joined within the window and has
    no clean message yet. Joins come from service messages and
    `chat_member` updates;
  - clean-up: when the action is ban or mute, the spammer's other messages in
    the window are deleted (tg-spam's "aggressive cleanup", on by default
    here).
- **State file (`store.rs`).** Trust is mirrored to SQLite (`rusqlite`,
  bundled, WAL) in `ANTISPAM_DATA_DIR/antispam.db`, with schema versions in
  `PRAGMA user_version`.
  - The Docker image sets `ANTISPAM_DATA_DIR=/data` and declares a volume.
    `compose.yaml` mounts a named volume there.
  - Without a data directory, or if it cannot be opened, the bot logs
    `store_unavailable` and keeps state in memory.
  - History is not persisted: it is short-lived by design.
- **Engine (`shield.rs`).** History, trust and the classifier form one pure
  function, `Shield::inspect`. It can be unit-tested and replayed against chat
  exports. The Telegram glue moved to `src/bot/`.

## Consequences

- In the replay, spam caught rose from 21% to 67% and false positives fell
  from 1.8% to 0.26%. The remaining misses are `@randombot` trigger messages,
  which a later meta-check addresses.
- After a fresh install every member is untrusted. Regulars need three
  messages before their links stop being checked by the model.
- Deployments without a volume lose trust on every restart. The bot still
  works, just with more model checks.
- One more dependency compiles C (bundled SQLite). Native runners build both
  architectures, so release builds are not slowed by emulation.
- Abnormal-spacing detection from tg-spam was evaluated and not adopted: on
  Russian/English chats it flagged 1.5–5.5% of normal messages.
