# 9. Admin feedback: commands, learned samples and reports

- Status: accepted
- Date: 2026-10-24

## Context

Automatic rules will always miss some spam and catch some normal messages.
Without feedback, admins delete misses by hand and the same wave keeps
coming. They also never learn about wrong deletions. tg-spam closes this loop:
`/spam` bans and adds a sample, unbanning adds a ham sample, and an admin
group gets notifications with buttons.

## Decision

- **Commands**, as a reply, from admins, anonymous admins or the owner:
  `/spam` (delete with clean-up, ban, learn), `/ban` (no learning) and
  `/warn`. Warnings fade after a quiet month; `ANTISPAM_WARN_LIMIT` (3) of
  them mean a ban. Commands are explicit admin actions, so they ignore
  dry-run.
- **Learned samples (`samples.rs`).** We compare word sets by cosine
  similarity with threshold 0.5, tg-spam's default. All spam of the replayed
  chat was treated as `/spam`-ed samples, then compared with 10,490 normal
  messages (chat and Telegram dataset). Thresholds 0.4–0.7 matched none of
  the normal messages; 0.5 matched 92% of the spam. We chose this over a
  second trained model (Bayes) because it is transparent, needs no minimum
  number of samples, and one `/spam` already works.
  - Spam samples apply to untrusted senders.
  - A ham sample ("not spam") overrules soft verdicts on similar messages:
    learned spam, shape limits, job offers and the model. Rules that people
    never trip by accident still apply.
- **Reports** go to `ANTISPAM_REPORT_CHAT`, or to the owner in private.
  - Each report shows chat, sender, reason, score, outcome and text, as plain
    text so names cannot inject markup.
  - "Not spam" unbans or unmutes the sender: for a mute, it restores the
    group's default permissions; for a channel, it calls
    `unbanChatSenderChat`. It also trusts the sender and learns the text as
    ham.
  - "Ban" is shown when the action only deletes. It bans, cleans up and
    learns the text as spam.
  - Only the owner or an admin of the reported chat may press the buttons.
- **Storage:** samples (at most 5,000 per kind), warnings and reports (newest
  1,000) are a second schema migration in `antispam.db`. `Store` became a
  cloneable handle shared by all components.

## Consequences

- The bot improves with use, without retraining the model.
- A deleted message cannot be restored. "Not spam" only frees the sender and
  prevents repeats.
- Private reports need the owner to press Start in the bot once. Until then
  sending fails, and the bot logs `report_failed`.
- Startup calls `getMe` to learn the bot's username, so `/spam@OtherBot`
  is ignored. A wrong token now fails at startup with a clear error.
