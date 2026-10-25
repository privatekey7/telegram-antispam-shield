# 3. Chat allowlist and owner-restricted status

- Status: accepted
- Date: 2026-06-23

## Context

The bot is designed to be forked and self-hosted: each operator runs their own
instance with their own `BOT_TOKEN`. By default a Telegram bot can be added to
**any** chat by **anyone**, and `getUpdates` then delivers every message from
those chats to the operator's instance.

For a bot whose username is public this creates real problems, even though there
are no remote-control commands an attacker could abuse:

- **Resource abuse (denial-of-wallet).** Every message in every chat the bot
  sits in is classified on the operator's host. Strangers adding the bot to
  their groups burn the operator's CPU/RAM/bandwidth and can exhaust free-tier
  quotas.
- **Privacy.** Moderation requires privacy mode off, so the instance receives
  and logs metadata for messages of third parties the operator never intended
  to serve.
- **Configuration disclosure.** `/status` previously replied to anyone in a
  private chat, exposing the instance's configuration.

Restricting by *sender* user ID does not help, because the bot has no
privileged commands — the unit that needs scoping is the **chat**.

## Decision

Add opt-in, environment-driven access control (settings stay separate from
logic, consistent with ADR 0001):

- `ANTISPAM_ALLOWED_CHATS` — comma/space/semicolon-separated chat IDs. When set,
  `Settings::chat_allowed` permits only those chats; the allowlist is checked
  **before** any message processing. When unset, every chat is allowed
  (backward-compatible default).
- `ANTISPAM_LEAVE_UNKNOWN_CHATS` — when true, the bot calls `leave_chat` on any
  chat not on the allowlist instead of silently ignoring it.
- `ANTISPAM_OWNER_ID` — when set, only this user receives `/status` output.

Telegram cannot prevent a public bot from being added to a group from the API
side (that is a BotFather `/setjoingroups` toggle which would disable groups
entirely), so enforcement lives in the bot: detect the unauthorized chat and
ignore or leave it.

## Consequences

- Operators with a public bot username can scope the bot to known chats and
  avoid serving — and logging — strangers' traffic.
- Defaults are unchanged, so existing deployments keep working with no config.
- The allowlist uses numeric chat IDs (groups are negative); operators read them
  from logs or `/status`, which is documented in the README.

## Addendum (2026-10-25): the bot is now a shared resource

Security audit 2026-10-24, findings M2 and M3.

The context above says the bot "has no privileged commands". Since ADR 0009
that is no longer true: `/spam`, `/ban`, `/warn` and the report buttons make
the bot delete, ban and unban with its own admin rights, and `/spam` feeds the
learned samples. Without an allowlist, the admins of any group the bot was
added to share those powers, and any such group can send the owner reports
(with the group's title and the spammer's text) and push real reports out of
storage. So:

- **Rights per action.** The bot must not do for someone what they could not
  do alone (confused deputy). Banning actions and "Not spam" (which unbans)
  need the owner or an admin with `can_restrict_members`; the creator has it.
  `/warn` needs the same while `ANTISPAM_WARN_LIMIT` > 0, since any warning may
  be the one that bans, and only `can_delete_messages` when warnings never
  ban. The policy is pure code in `access.rs`, tested without Telegram.
- **Anonymous admins are refused.** A message sent as the group does not say
  which admin wrote it, so their rights are unknown: an admin who may only pin
  messages would get full powers. They get a hint to turn anonymity off for
  the command, and can still press report buttons, which always come from a
  person. A confirmation button (as some moderation bots do) would keep
  anonymity, at the cost of an extra flow.
- **Demoted admins.** The cached admin list (now with rights) turns away
  non-admins without an API call. When the cache allows a request, the bot
  confirms it with `getChatMember`, so a demoted admin loses access at once.
  A request from someone not in the cache refreshes the list at most once a
  minute, so a new admin waits a minute at most and strangers typing `/ban`
  cannot make the bot hammer the API. If Telegram cannot be asked, the request
  is refused.
- **The allowlist fails closed when set.** If `ANTISPAM_ALLOWED_CHATS` (or
  `ANTISPAM_OWNER_ID`) is set but an entry is not a number, the bot refuses to
  start instead of silently allowing every chat.
- **`/status` without an owner** shows nobody the configuration; it only says
  that the bot is running.
- **Reports** come only from chats that pass the allowlist (every chat while
  it is unset), at most 20 per chat per 10 minutes, and stay plain text (no
  parse mode), since titles and texts from foreign chats are untrusted. The
  limit is per chat: many hostile chats can still flood the owner and push
  stored reports out. Only the allowlist stops that.
- **An unset allowlist stays the default**, now with a startup warning, and
  the README, `.env.example` and hosting guides strongly recommend setting it.
  Failing closed on an empty allowlist would be safer, but it would stop every
  existing deployment without one, and the default is the owner's call.
