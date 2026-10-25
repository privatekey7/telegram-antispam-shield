# 10. Own polling loop that repairs unreadable updates

- Status: accepted
- Date: 2026-10-24

## Context

During the first live test, `/spam` sent as a reply to a message from
before the group became a supergroup did nothing. The bot logged nothing and
did not even delete the command.

Telegram delivers such a reply as `external_reply`: a reply to a message in
another chat. teloxide-core 0.13, the latest release, parses
`ExternalReplyInfo` with a flattened enum that has a variant for every kind of
media but none for plain text. Parsing it fails, and because `MessageKind` is
an untagged enum ending in `Empty {}`, the whole message silently becomes an
empty one. Its text and caption are lost before any handler runs. The same
happens to any message that replies to a text message from another chat
("Reply in another chat"), so such messages were neither moderated nor read as
commands.

## Decision

- **Poll with `getUpdates` ourselves (`src/bot/polling.rs`).** The poller
  sends `getUpdates` through teloxide's own `JsonRequest`, with a payload whose
  output is raw JSON. It then drops every `external_reply` that does not parse
  as `ExternalReplyInfo`, and only after that parses the update. The
  dispatcher runs on it through `dispatch_with_listener`. Otherwise it works
  like teloxide's polling:
  - it deletes a leftover webhook at startup;
  - it accepts the allowed-updates hint;
  - it backs off after errors and honours `retry_after`;
  - on shutdown it confirms the last offset.
- **Parse updates from text, not `Value`.** teloxide's `Update` deserializer
  turns anything other than a string source into `UpdateKind::Error`.
- **Log what still gets lost.** If the raw JSON had text or a caption but the
  parsed message is empty, the bot logs `unreadable_message`. A future gap
  then shows up in the logs instead of passing silently.
- **Commands explain themselves.** A command that is not a reply, or that
  replies to a message out of reach (the dropped `external_reply`), now shows a
  20-second hint instead of vanishing. So does a command that replies to a
  bot's message.

Alternatives we rejected:
- A fork or a vendored copy of teloxide-core: too much to maintain for a
  one-field fix.
- A `Requester` wrapper: it would have to forward about 150 methods.
- Waiting for an upstream fix: there is no issue or release for it yet.

## Consequences

- Messages that reply to other chats are moderated like any other message.
- The poller remembers which messages lost an `external_reply` (a bounded
  set), so `ANTISPAM_BLOCK_FORWARDS` still sees text quotes. The quoted text
  itself is not moderated.
- Every `getUpdates` carries the allowed-updates list, not only the first:
  Telegram keeps the old subscription until a call with the new one
  succeeds, so a failed first call after an upgrade would leave new update
  types (`edited_message`) off until the next restart.
- The test `teloxide_alone_loses_the_text` fails once teloxide fixes the bug
  upstream. That is the signal to delete the repair and return to
  `Dispatcher::dispatch`.
