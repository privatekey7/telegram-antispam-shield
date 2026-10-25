# 8. Shape checks and Combot Anti-Spam lookups

- Status: accepted
- Date: 2026-10-24

## Context

After trust and history (ADR 0007), the replayed spam-ridden chat still had
73 missed messages, all `@randombot` triggers. A business bot answered each one
with an emoji grid and link buttons. tg-spam has a family of "meta checks"
(emoji count, mention-only, links limit, mixed-language words, abnormal
spacing, forwards). It also queries the Combot Anti-Spam (CAS) database of
known spammers by default.

The text model also misfired on very short messages from new members: "лс",
"0/2", "всем привет".

## Decision

- **Shape checks (`meta.rs`)** run for untrusted members after the newcomer
  rule and before job offers and the model. Defaults come from measurements
  on the Telegram and mixed spam datasets and the replayed chat:

  | Check | Normal messages hit | Spam hit | Default |
  |---|---|---|---|
  | message is only `@…bot` | 0 | 73 (chat) | on |
  | 2+ words mixing Cyrillic and Latin | 0.01–0.03% | 15% (Telegram set) | on (2) |
  | 10+ emoji | 0–0.01% | 36% (chat) | on (10) |
  | 5+ emoji | 1.1% (chat) | — | rejected |
  | abnormal spacing | 1.5–5.5% | — | rejected |
  | 3+ links | ~0% | ~0% | opt-in |

  Blocked scripts, stop words, forwards and link/mention limits depend on the
  community, so they are opt-in.
- **Minimum length for the model.** Messages under `ANTISPAM_MODEL_MIN_CHARS`
  (20) without a link skip the model. Recall on the Telegram set is the same
  (3,897/3,897). Model false positives in the chat drop by a quarter.
- **CAS** is on by default, as in tg-spam. Only untrusted members are looked
  up: on each message (cached for an hour) and on join. A listed member who
  joins is banned or muted at once when the action allows it. Timeouts and
  errors count as "not listed" and are not cached.

## Consequences

- Replay: 99.1% of spam caught, 0.18% false positives. Before ADR 0007 it was
  21% and 1.8%.
- CAS sends the numeric user IDs of untrusted members to `api.cas.chat`. The
  README says so and shows how to turn it off. The container now talks to two
  hosts.
- In the replayed chat CAS knew 1 of 33 spam accounts. Most were deleted
  accounts, which seem to drop out of CAS. Its value is catching known
  spammers on their first message in a new chat; it is not the main defence.
- An HTTP client (`reqwest`, already pulled in by teloxide) is now a direct
  dependency.

## Addendum (2026-10-25): CAS removed

In October 2026 `api.cas.chat` began to answer every lookup with
`401 API key required`. Keys are tied to a Combot account. Since then CAS
caught nothing, and a slow or failing lookup still cost time on each
untrusted message. In the replayed chat it had found 1 of 33 spam accounts.

We removed CAS in 0.3.1 instead of adding an API key setting: the bot should
not depend on a third-party service or account. The lookup on messages and
on joins, the `cas_banned` reason and the `ANTISPAM_CAS_*` settings are gone.
The settings are now ignored. The container talks only to `api.telegram.org`,
and `reqwest` is no longer a direct dependency.
