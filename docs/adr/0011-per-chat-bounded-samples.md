# 11. Learned samples per chat, with size limits

- Status: accepted
- Date: 2026-10-25
- Amends: [ADR 0009](0009-admin-feedback-loop.md)

## Context

ADR 0009 stored learned samples in one list shared by all chats. A security
audit (2026-10-24, findings H1 and H2) showed two problems with that.

- **Cross-chat control.** An admin of any chat the bot is in could `/spam` a
  harmless text and make the bot delete similar messages, and ban their
  authors, in every other chat. 5,000 junk `/spam` also pushed out the
  samples other chats had learned. Without `ANTISPAM_ALLOWED_CHATS`, anyone
  can become such an admin by creating a group.
- **Memory.** Only the number of samples was capped (5,000 per kind), not
  their size, and duplicates were stored again. About 2,300 `/spam` on one
  4,096-char message of short distinct words passed the container's 128 MB
  limit. The samples are reloaded on start, so the bot crashed after every
  restart. Matching took about 0.3 s per message at 5,000 samples.

## Decision

- **Samples belong to a chat.** `/spam`, "Not spam" and "Ban" in a report
  learn for the chat the message came from. A message is compared only with
  its own chat's samples. The rules of ADR 0009 are unchanged otherwise:
  cosine 0.5, at least 3 words, spam samples for untrusted senders only, ham
  overruling soft verdicts.
- **Size.** Only the first 800 chars (the model's limit in `features.rs`) and
  at most 128 distinct words of a text are learned or compared. 800 chars of
  prose hold about 110–130 words, so the word cap only bites on word salad.
  In memory a sample is a sorted list of 64-bit word hashes, at most about
  1 KB; two samples are compared by merging the lists.
- **No duplicates.** A text whose word set a chat already has is not learned
  again. The store keeps the sorted word list as the text, with
  `UNIQUE (chat_id, kind, text)` as a second guard.
- **Counts.** At most 1,000 samples per chat and kind; the chat's oldest go
  first. At most 5,000 per kind in total; when full, the oldest sample of the
  chat holding the most goes, so a few large or hostile chats cannot push out
  a small chat's samples while they hold more.
- **Budget.** Worst case 2 kinds × 5,000 × ~1.1 KB ≈ 11 MB in memory. A
  message is compared with at most 1,000 samples of each kind: about half a
  million integer comparisons, around a millisecond.
- **Loading** applies the same limits. Rows that are too short, duplicated or
  over a limit are deleted, so the file cannot grow past what memory holds.
- **Migration 3** renames the old table to `samples_unscoped` and creates a
  new `samples` table with `chat_id`. Old samples have no chat and any of them
  may have been planted from another chat, so the bot never loads them. They
  are kept rather than deleted; an owner who trusts them can copy them into a
  chat by hand:
  `INSERT OR IGNORE INTO samples (chat_id, kind, text) SELECT <chat id>, kind, text FROM samples_unscoped;`
  The next start then trims them to the limits above.
- We did not add a rate limit on `/spam`: with deduplication and the caps a
  flood of commands only cycles one chat's own samples.

## Consequences

- An admin's feedback no longer reaches other chats. A wave of spam that hits
  several chats has to be taught in each of them.
- After the upgrade every chat starts with no learned samples.
- Very long messages are compared by their start only, as the model already
  does. A spammer could hide a learned text after 800 chars of filler; repeats,
  campaigns and the link rules still apply to such messages.
- `/status` shows the number of spam samples in all chats.
