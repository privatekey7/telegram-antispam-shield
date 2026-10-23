# 4. Anti-flood with in-memory sliding windows

- Status: accepted
- Date: 2026-10-24

## Context

The classifier (ADR 0002) scores each message on its own. During a public
review of the bot, a user posted the same harmless message many times in a row
and the bot let every copy through: each one was "not spam" in isolation. A
burst of stickers or media was invisible too, because only messages with text
were inspected.

Flooding is a property of a *sequence* of messages from one user, so it needs
per-user state, which the bot did not keep.

## Decision

Add a pure `flood` module (`FloodGuard`) that keeps, per `(chat, user)`, a short
sliding window of recent messages and an offence counter. It runs before the
spam classifier and sees every message type.

- **Rate rule:** more than `max_messages` (default 6) messages within
  `rate_window` (10 s). Messages of one album (`media_group_id`) count as one,
  so sending ten photos at once is not flooding.
- **Duplicate rule:** `max_duplicates` (3) messages with the same normalized
  text within `duplicate_window` (120 s). Normalization lowercases, keeps only
  letters and digits, and collapses repeated characters, so "Привееет!!!" equals
  "привет". Texts shorter than 3 characters are not compared, because replies
  like "+" are legitimately repeated; the rate rule still covers them. Fuzzy
  matching (shingles/Jaccard) was rejected for now: exact matching after
  normalization is predictable and has almost no false positives.
- **Action:** delete every message of the burst (`deleteMessages`, so earlier
  copies disappear too) and mute the user. Mutes escalate per offence
  (default 10m → 1h → 1d, last step repeats); the counter resets after a day
  without offences. A short notice is posted and deleted after 30 s. This is
  separate from `ANTISPAM_ACTION`: a flooder is usually a real person, so a
  temporary mute fits better than a ban.
- **Exemptions and safety:** admins are exempt (same setting as for spam),
  dry-run only logs, and a missing "Ban users" right degrades to delete-only
  with a warning.
- **State lives in memory.** It is lost on restart, which only resets offence
  counters. A periodic sweep drops idle users so memory stays bounded. No
  database is added for this.
- The feature is behind `ANTISPAM_FLOOD_ENABLED` (default on) and all limits are
  environment variables, consistent with ADR 0001.

`FloodGuard` takes `now` as a parameter instead of reading the clock, so every
rule is covered by fast unit tests.

## Consequences

- Repeated messages, sticker storms and rapid-fire posting are stopped within a
  few messages, and the chat is cleaned up retroactively.
- The bot must be granted "Ban users" for mutes to work; the README says so.
- The admin list is now consulted for every group message, not only text ones;
  it stays cached (`ANTISPAM_ADMIN_CACHE_TTL`), so API calls do not grow.
- A restart forgets offence history; acceptable for a single small instance.
  The bot already runs as one replica (long polling allows only one consumer
  per token), so in-memory state is never split between instances.
