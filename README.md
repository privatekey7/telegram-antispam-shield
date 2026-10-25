# 🛡 Telegram Anti-Spam Shield

🌍 **English** · [Русский](README.ru.md)

A lightweight Telegram bot that **automatically detects and deletes spam** (in
Russian and English) using a small built-in machine-learning model. It is fast,
free to run, and designed so that **anyone can host it in ~10 minutes** on any
machine with Docker — a server, a Raspberry Pi, a NAS or a container platform.

[![CI](https://github.com/privatekey7/telegram-antispam-shield/actions/workflows/ci.yml/badge.svg)](https://github.com/privatekey7/telegram-antispam-shield/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

---

## What it does

- 🤖 Classifies every group message as **spam** or **not spam** with a small ML model.
- 🗑 **Deletes** spam automatically (and can optionally **ban** or **mute** the sender).
- 🌊 **Anti-flood:** stops users who post too fast or repeat the same message —
  deletes the whole burst and mutes them (10 min → 1 h → 1 day for repeat offences).
- 💼 **Job offers are spam:** "remote work, 300$ a day, DM me" is removed by
  default — real chats don't recruit in the open.
- 🕵️ **Sees through tricks:** invisible characters inside words, links hidden
  under text, inline buttons that only automation can attach, and spam posted
  on behalf of a channel.
- 🇷🇺 🇬🇧 Trained on real Russian + English spam.
- 🔎 **Shape checks** (ideas from tg-spam, defaults chosen on data): mixed
  Cyrillic/Latin words, emoji walls, `@somebot`-only triggers, plus opt-in
  stop words, blocked scripts, forward and link/mention limits. Known
  spammers are looked up in the [CAS](https://cas.chat) database.
- 👮 **Admin tools:** reply `/spam` once and the whole reworded wave is
  deleted from then on; `/ban`, `/warn` with auto-ban; reports with
  **Not spam** / **Ban** buttons that undo mistakes and teach the bot.
- 🤝 **Trusts regulars:** after three normal messages a member's links and
  slang are no longer second-guessed by the model; newcomers posting links,
  slow repeaters and multi-account spam waves are caught instead.
- 🪶 Tiny footprint: a few tens of MB of RAM, no open ports, one small SQLite file.
- 📦 Prebuilt images for amd64 and arm64 — or let an AI agent install it for you.
- 🔌 No external AI API, no per-message cost. The model is **built into the bot**.
- 🛟 Safe by default: admins are never moderated, and a **dry-run mode** lets you
  watch it work before it deletes anything.

### How good is it?

Measured on a held-out test set of public spam data:

| Metric | Value |
|---|---|
| Accuracy | **97.0%** |
| ROC-AUC | **0.994** |
| Spam caught (recall) | **93.6%** |
| False-positive rate (legit deleted) | **~1.0%** |
| Model size | **4.2 MB** (embedded) |

The decision threshold is deliberately tuned to **rarely delete legitimate
messages** — it's better to miss some spam than to remove a real message.

---

## 🚀 Install in 10 minutes

You will: (1) create a bot, (2) start it on a machine of your choice, (3) add
it to your group. Follow the steps in order.

### Step 1 — Create your bot and get a token

1. In Telegram, open [@BotFather](https://t.me/BotFather).
2. Send `/newbot` and follow the prompts (choose a name and a username).
3. BotFather gives you a **token** that looks like
   `123456789:AAExxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx`. **Copy it** and keep it
   secret — anyone with it controls your bot.
4. Send `/setprivacy` → pick your bot → choose **Disable**. This lets the bot
   see all group messages so it can check them for spam. *(Important!)*

### Step 2 — Start the bot

**Option A — let an AI agent do it.** Give your coding agent (Claude Code,
Codex, Cursor…) access to the machine and say:

> Install the Telegram bot from https://github.com/privatekey7/telegram-antispam-shield
> by following its llms-install.md.

The agent installs, configures and verifies everything and asks you only for
the steps in Telegram. Guide: [llms-install.md](llms-install.md).

**Option B — Docker, by hand.** On any always-on machine with Docker:

```bash
mkdir antispam-shield && cd antispam-shield
base=https://github.com/privatekey7/telegram-antispam-shield/releases/latest/download
curl -fsSLO "$base/compose.yaml"
curl -fsSL "$base/env.example" -o .env && chmod 600 .env
nano .env                 # set BOT_TOKEN=<token from Step 1>
docker compose up -d
docker compose logs -f    # "bot_started" means it's live 🎉
```

**Other ways:** `docker run`, Railway / Render / Fly.io and other container
platforms, a build from source with systemd — see
[docs/hosting.md](docs/hosting.md).

> 🔒 Your token stays on your machine, in `.env` (or in your platform's
> variables). It is never committed and is not part of the image.

### Step 3 — Add the bot to your group

1. Open your Telegram **group** → **Add members** → add your bot.
2. Make the bot an **administrator** with at least the **Delete messages**
   permission. *(Enable **Ban users** too, so the bot can mute flooders and
   ban/mute spammers.)*
3. Done. The bot now removes spam automatically.

**Test it:** in the group, send something spammy like
`ЗАРАБОТОК ОТ 5000$ В ДЕНЬ! Пиши в личку` — it should be deleted within a second.

> 🧪 Want to watch before it deletes? Set `ANTISPAM_DRY_RUN=true` in `.env`
> and run `docker compose up -d`. The bot will only log what it *would* delete.
> Set it back to `false` when you're confident.
>
> 🔐 Lock the bot to your groups with `ANTISPAM_ALLOWED_CHATS` —
> [how to find the group ID](docs/hosting.md#restricting-the-bot-to-your-chats).
> Every bot's username is public: without the list anyone can add the bot to
> their group, use your server and send you reports from any number of
> chats.

---

## ⚙️ Configuration

All settings are **environment variables**: put them in `.env` next to
`compose.yaml` (or in your platform's variables), then `docker compose up -d`.
Only `BOT_TOKEN` is required; everything else has sensible defaults.

| Variable | Default | Description |
|---|---|---|
| `BOT_TOKEN` | — | **Required.** Token from @BotFather. |
| `ANTISPAM_ACTION` | `delete` | `delete`, `ban`, or `mute` the spammer. |
| `ANTISPAM_DRY_RUN` | `false` | If `true`, only log detections — never delete. |
| `ANTISPAM_THRESHOLD` | *(model default ≈ 0.82)* | Spam probability cutoff `0.0–1.0`. Higher = stricter (fewer deletions). |
| `ANTISPAM_BLOCK_INVITE_LINKS` | `true` | Treat `t.me/+…` / `joinchat` links as spam. |
| `ANTISPAM_BLOCK_HIDDEN_CHARACTERS` | `true` | Treat invisible characters inside words (`act​ive`) as spam. People don't type them; filter dodgers do. |
| `ANTISPAM_BLOCK_INLINE_KEYBOARDS` | `true` | Treat inline buttons on a member's message as spam. A person can't attach buttons; only automation such as a business bot can (messages sent via inline bots are not affected). |
| `ANTISPAM_BLOCK_JOB_OFFERS` | `true` | Treat job and income offers (remote work, side income, "earn $X a day") as spam when they mention pay or ask to DM. Talking about work is fine. |
| `ANTISPAM_TRUST_AFTER` | `3` | Clean messages after which a member is trusted: the model, job-offer and newcomer-link checks stop for them (buttons, hidden characters, invite links and repeats still apply). `0` checks everyone. |
| `ANTISPAM_TRUST_MIN_CHARS` | `10` | Shorter messages ("+", "hi") don't count toward trust. |
| `ANTISPAM_BLOCK_NEWCOMER_LINKS` | `true` | Treat a link from someone who joined within the history window and hasn't written a normal message yet as spam. |
| `ANTISPAM_BLOCK_CAMPAIGNS` | `true` | Treat the same long text (40+ characters) from several untrusted accounts as spam and delete every copy. |
| `ANTISPAM_REPEAT_LIMIT` | `3` | Spam when one member posts the same text (15+ characters) this many times within the history window, even slowly. `0` turns it off. |
| `ANTISPAM_HISTORY_WINDOW_SECS` | `86400` | How long messages and joins are remembered for the rules above. |
| `ANTISPAM_CLEANUP_ON_BAN` | `true` | When the action is `ban` or `mute`, also delete the spammer's other messages from the history window. |
| `ANTISPAM_DATA_DIR` | `/data` in Docker | Folder for `antispam.db`, which keeps trusted members across restarts. Unset = memory only. |
| `ANTISPAM_HEARTBEAT_FILE` | `/tmp/antispam-heartbeat` | Heartbeat behind the image's `HEALTHCHECK` ([docs/hosting.md](docs/hosting.md#health)). |
| `ANTISPAM_MODEL_MIN_CHARS` | `20` | Messages shorter than this and without links skip the model ("лс", "всем привет" carry too little signal). No recall loss on the test data. |
| `ANTISPAM_BLOCK_BOT_MENTIONS` | `true` | Treat a message that is only a bot's `@username` (`@gjwgjglkewbot`), the trigger of business-bot spam, as spam. |
| `ANTISPAM_MIXED_SCRIPT_WORDS` | `2` | Spam when this many words mix Cyrillic and Latin letters (`зaрaботок` with Latin `a`). `0` = off. |
| `ANTISPAM_EMOJI_LIMIT` | `10` | Spam when a message has this many emoji. `0` = off. |
| `ANTISPAM_CAS_ENABLED` | `true` | Look up untrusted members in the [Combot Anti-Spam](https://cas.chat) database of known spammers (on their messages and when they join). Sends their numeric user ID to `api.cas.chat`; results are cached for an hour; a slow or failing API counts as "not listed". |
| `ANTISPAM_CAS_TIMEOUT_SECS` | `5` | Give up on a CAS lookup after this long. |
| `ANTISPAM_BLOCKED_SCRIPTS` | _(none)_ | Comma-separated writing systems whose messages are spam: `chinese`, `japanese`, `korean`, `arabic`, `hebrew`, `devanagari`, `thai`, `greek`, `armenian`, `georgian`. |
| `ANTISPAM_STOP_WORDS` | _(none)_ | Comma-separated phrases (case-insensitive) that make a message spam. |
| `ANTISPAM_BLOCK_FORWARDS` | `false` | Treat forwards, quotes from other chats and stories from untrusted members as spam. |
| `ANTISPAM_LINK_LIMIT` | `0` | Spam when a message has this many links. `0` = off. |
| `ANTISPAM_MENTION_LIMIT` | `0` | Spam when a message has this many `@mentions`. `0` = off. |
| `ANTISPAM_DELETE_SERVICE_MESSAGES` | `false` | Delete "X joined" / "X left" notices. |
| `ANTISPAM_REPORT_CHAT` | _(owner)_ | Chat ID that receives a report about every deleted spam message, with **Not spam** / **Ban** buttons. Unset = reports go to `ANTISPAM_OWNER_ID` in private (press Start in the bot first); neither set = no reports. |
| `ANTISPAM_WARN_LIMIT` | `3` | `/warn`s within a month after which the member is banned. `0` = warnings never ban. |
| `ANTISPAM_SIMILARITY_THRESHOLD` | `0.5` | How close (cosine of word sets, `0–1`) a message must be to a `/spam`-ed one to be deleted automatically. |
| `ANTISPAM_EXEMPT_ADMINS` | `true` | Never moderate group admins. Posts of the group itself (anonymous admins, the linked channel) are always exempt. |
| `ANTISPAM_ADMIN_CACHE_TTL` | `3600` | Seconds to cache each group's admin list and rights (used to exempt admins and to turn away non-admins). Commands and report buttons always re-check the admin's rights with Telegram. |
| `ANTISPAM_ALLOWED_CHATS` | _(all chats)_ | Comma-separated numeric chat IDs the bot is allowed to moderate. When set, every other chat is ignored and sends no reports. **Strongly recommended**: without it, strangers can add the bot, use your hosting, send you reports and teach it samples. An entry that is not a number (such as `@mygroup`) stops the bot at startup. |
| `ANTISPAM_LEAVE_UNKNOWN_CHATS` | `false` | When `true`, the bot leaves any chat not in `ANTISPAM_ALLOWED_CHATS` instead of just ignoring it. |
| `ANTISPAM_OWNER_ID` | _(none)_ | Your numeric Telegram user ID. Only you see the `/status` details in a private chat; anyone else (everyone, when unset) only learns that the bot is running. The owner may also use commands and report buttons in every allowed chat. Get it from [@userinfobot](https://t.me/userinfobot). |
| `ANTISPAM_FLOOD_ENABLED` | `true` | Turn anti-flood on/off. |
| `ANTISPAM_FLOOD_MAX_MESSAGES` | `6` | Flood when a user sends **more** than this many messages within the window. An album counts as one. |
| `ANTISPAM_FLOOD_WINDOW_SECS` | `10` | Window for the message-rate rule, seconds. |
| `ANTISPAM_FLOOD_MAX_DUPLICATES` | `3` | Flood when this many messages with the same text arrive within the duplicate window (case, punctuation and stretched letters are ignored; texts under 3 characters are not compared). |
| `ANTISPAM_FLOOD_DUPLICATE_WINDOW_SECS` | `120` | Window for the repeated-text rule, seconds. |
| `ANTISPAM_FLOOD_MUTE_STEPS` | `10m,1h,1d` | Mute length for the 1st, 2nd, 3rd… offence (`s`/`m`/`h`/`d`); the last step repeats. |
| `ANTISPAM_FLOOD_STRIKE_RESET_SECS` | `86400` | Offence counter resets after this long without a new offence. |
| `ANTISPAM_FLOOD_NOTIFY` | `true` | Post a short "muted for flooding" notice in the chat. |
| `ANTISPAM_FLOOD_NOTIFY_TTL_SECS` | `30` | Delete that notice after this many seconds. |
| `RUST_LOG` | `info` | Log level: `error`/`warn`/`info`/`debug`. |
| `MODEL_PATH` | *(unset)* | Path to a custom-trained model (advanced). |

See [`.env.example`](.env.example) for a copy-paste template.

---

## 👮 Admin commands and reports

Reply to a message in the group with:

| Command | What happens |
|---|---|
| `/spam` | Deletes the message and the author's recent messages, bans the author and **learns** the text: similar messages (even reworded) are deleted automatically from now on. |
| `/ban` | The same without learning. |
| `/warn` | Deletes the message and warns the author (a notice for a minute). `ANTISPAM_WARN_LIMIT` warnings in a month mean a ban. |

Commands work for `ANTISPAM_OWNER_ID` and for group admins with the
**Ban users** right (the group's creator has it), also in dry-run mode: the
bot never bans for someone who could not ban alone. `/warn` needs the same
right while warnings can end in a ban; with `ANTISPAM_WARN_LIMIT=0` the
**Delete messages** right is enough. The bot asks Telegram for the admin's
current rights before acting, so a demoted admin loses access at once.

**Anonymous admins** (writing as the group) cannot use commands: Telegram
does not say which admin wrote the message, so their rights cannot be
checked. Turn off **Remain anonymous** for the command, or use the report
buttons, which always show who pressed them.

The command message itself is deleted. A command that is not a reply, or
replies to a message out of reach (such as one sent before the group became a
supergroup), gets a 20-second hint instead; so does a command from an admin
without the right. Commands never act on admins, the bot's owner or posts of
the group itself (anonymous admins, the linked channel): a reply to one of
them also gets a hint.

**Reports.** Set `ANTISPAM_OWNER_ID` (or `ANTISPAM_REPORT_CHAT`) and every
automatic deletion is reported with the reason, the sender and the text:

- **✅ Not spam** — unbans or unmutes the sender, makes them trusted and
  learns the text as fine, so similar messages pass;
- **🚫 Ban** (when the action only deletes) — bans, cleans up and learns the
  text as spam.

Both buttons need the owner or an admin of the reported chat with the **Ban
users** right. Each chat sends at most 20 reports per 10 minutes; the rest
are only logged.

Learned samples work only in the chat where they were taught, at most 1,000
per kind and chat. They live in `antispam.db` next to trust, warnings and
reports.

---

## 🧠 How it works

```
Incoming message
   │
   ├─ 0. Anti-flood  → too many messages / same text repeated?    → delete burst + mute
   │
   ├─ 1. History     → listed in CAS, same text 3× a day, same promo from other accounts,
   │                   looks like a /spam-ed message                → spam
   │
   ├─ 2. Rules       → buttons, hidden characters, invite links   → spam
   │     (trusted members stop here)
   ├─ 3. Rules       → newcomer links, stop words, scripts, @bot, mixed alphabets,
   │                   emoji, forwards, link/mention limits, job offers → spam
   │
   ├─ 4. ML model    → 20+ chars or a link: spam probability ≥ threshold? → spam
   │
   └─ otherwise: allow
```

- **What the rules see:** text or caption with invisible characters removed
  (`src/text.rs`), plus the targets of links hidden under words, so a
  `t.me/+…` invite behind innocent text is still caught.
- **Trust and history** (`src/trust.rs`, `src/history.rs`, ADR 0007): a
  member is trusted after 3 clean messages of 10+ characters. Messages and
  joins of the last 24 h are kept in memory to spot repeats, campaigns and
  newcomers. Trust is saved to `antispam.db` in `ANTISPAM_DATA_DIR` (a Docker
  volume), so a redeploy keeps it. The whole decision is one pure function,
  `Shield::inspect` (`src/shield.rs`).
- **Edited messages** are checked like new ones, so a harmless message edited
  into an ad is still caught. An edit is not a new message: it skips
  anti-flood, is not recorded for the repeat and campaign rules, earns no
  trust and runs no admin commands. Trusted members and admins stay exempt.
- **Anti-flood** (`src/flood.rs`) keeps a short in-memory window of each user's
  recent messages per chat. It sees every message type, including stickers and
  media. Offence counters live in memory and reset when the bot restarts.
- **Feature extraction** turns text into hashed word + character n-grams
  (`src/features.rs`). Character n-grams give robustness to obfuscation and
  multilingual text.
- **The model** is a logistic-regression classifier stored as a 4 MB binary and
  embedded in the executable, scored in pure Rust (`src/model.rs`).
- Training happens offline in Python (`scripts/train_model.py`). A parity test
  (`tests/parity.rs`) guarantees the Rust scorer matches the Python trainer.

Architecture decisions are recorded in [`docs/adr/`](docs/adr/).

---

## 🛠 Development

```bash
# Run tests, linter and formatter (needs Rust stable)
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check

# Run locally (reads BOT_TOKEN from a local .env)
cp .env.example .env   # then put your token in .env
cargo run
```

Releasing: bump `version` in `Cargo.toml`, the `IMAGE_TAG` default in
`compose.yaml` (on a minor/major bump) and move `Unreleased` in `CHANGELOG.md`
to the new version; after merging, push the tag `vX.Y.Z`. The Release workflow
checks they match, publishes the amd64 + arm64 image to GHCR and creates the
GitHub Release. Auto-deploying your own fork: [docs/auto-deploy.md](docs/auto-deploy.md).

## 🔁 Retraining the model (optional)

The bot ships with a ready model, so this is only for advanced users.

```bash
pip install -r scripts/requirements.txt
python scripts/train_model.py     # downloads datasets, trains, writes model/spam_model.bin
cargo test                        # parity test confirms Rust matches the new model
```

Datasets used: [`alt-gnome/telegram-spam`](https://huggingface.co/datasets/alt-gnome/telegram-spam)
and [`mshenoda/spam-messages`](https://huggingface.co/datasets/mshenoda/spam-messages).

---

## ❓ Troubleshooting

| Symptom | Fix |
|---|---|
| Bot doesn't delete anything | Make it a group **admin** with **Delete messages**; disable privacy via `/setprivacy` in BotFather. |
| Logs show `delete_failed` | The bot lacks delete rights — fix its admin permissions. |
| It deleted a legitimate message | Raise `ANTISPAM_THRESHOLD` (e.g. `0.9`) or report the example as an issue. |
| Want to test safely | Set `ANTISPAM_DRY_RUN=true`. |
| `TerminatedByOtherGetUpdates` in logs | Another copy of the bot uses the same token — stop it. Only one copy may run. |
| Mutes don't work | Grant **Ban users**; mutes need a supergroup. |

---

## License

[MIT](LICENSE) — free to use, fork, and modify.
