# 12. Closed by default, and a busy chat may not stall the rest

- Status: accepted
- Date: 2026-10-25
- Amends: [ADR 0003](0003-chat-access-control.md),
  [ADR 0007](0007-trust-history-and-state.md),
  [ADR 0009](0009-admin-feedback-loop.md)

## Context

A fresh security audit of v0.2.2 (findings A1–A16) found no critical
issues. Most findings were small and are fixed without a design question.
Five changed behaviour that earlier ADRs had fixed, so they are recorded
here.

- **Open by default (A7).** ADR 0003 kept "no allowlist = every chat" so
  existing installs would not break. A bot's username is public, so every
  install without a list shared its dispatcher, sample budget, report store
  and owner inbox with any stranger's group. Two other findings (A2, A5)
  were serious only because of this default.
- **One queue blocks all (A2).** teloxide gives each chat a worker with a
  bounded queue, and the dispatcher waits when a queue is full. That stops
  updates for every chat. One raid, a stranger's group or a hanging CAS API
  delayed moderation everywhere: 4.8 s for a single message in the audit's
  test.
- **Report buttons by number (A1).** A button carried only the report ID.
  IDs start over in memory mode, after a database reset, and in the save
  fallback. An old **Not spam** could then unban and trust a real spammer,
  in another chat, and say it had succeeded.
- **Campaign retro-delete (A4).** The second account posting a long text
  made the first copy go too. A troll could repost a newcomer's question and
  get it deleted.
- **Eternal trust (A16).** Three clean messages trusted a member forever.
  That trust skips the model, CAS and the campaign rule, which made dormant
  or sold accounts valuable.

## Decision

- **No allowlist means no chat.** Without `ANTISPAM_ALLOWED_CHATS` the bot
  starts, moderates nothing, and logs `chat_not_allowed` once per group with
  its ID. That is how an owner finds their group's ID without `getUpdates`.
  `ANTISPAM_ALLOW_ALL_CHATS=true` restores the old behaviour, with a startup
  warning. We did not make the bot refuse to start: with
  `restart: unless-stopped` that is a crash loop, and the log gives a
  first-time owner a shorter path to the ID. Either way no chat is served
  by accident.
- **Drop rather than wait.** The poller counts updates it handed over per
  chat. A shared `Backlog` releases one when the first step of the handler
  tree takes it. An update whose chat queue (256) is full, or that would
  pass 4,096 queued in total, is dropped and logged once per episode. A
  dropped message goes unmoderated in the busy chat. Waiting would delay all
  chats, so dropping is the lesser harm. A failed CAS lookup also pauses CAS
  for a minute, which removes the most common slow handler.
- **Buttons carry a token.** Callback data is `verb:<id>:<token>`, with a
  random 64-bit token stored with the report (migration 4). Lookups need
  both. Buttons from before 0.3 have no token; they match only rows saved
  before the migration (token 0), which are still the reports they were
  made for.
- **A wave needs three accounts.** Every untrusted copy is still spam, but
  earlier copies are deleted only once three accounts have posted the text.
  With two, only the newer copy goes: either account may be the victim.
- **Trust lapses after 90 silent days** (`ANTISPAM_TRUST_TTL_DAYS`, `0` =
  never). Activity is stored at most once a day per member. Untrusted
  members silent for 30 days, lapsed trusted ones and faded warnings are
  pruned on start and every sweep.

## Consequences

- Existing installs without a list stop moderating after the upgrade to
  0.3, until the owner adds the IDs from the log or opts in to every chat.
  The minor version bump keeps `0.2` users on the old behaviour until they
  choose to upgrade.
- Under a heavy raid, part of one chat's messages may go unchecked. Admins
  still see them and can act; other chats stay fast.
- A regular who disappears for three months is checked like a newcomer for
  three messages after returning.
- Two-account spam pairs leave their first copy, unless another rule
  catches it, until a third account confirms the wave.
