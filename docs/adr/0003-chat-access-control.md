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
