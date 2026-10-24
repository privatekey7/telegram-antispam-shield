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
- 🇷🇺 🇬🇧 Trained on real Russian + English spam.
- 🪶 Tiny footprint: a few tens of MB of RAM, no open ports, no database.
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
> 🔐 Bot username public? Lock the bot to your group with
> `ANTISPAM_ALLOWED_CHATS` — [how to find the group ID](docs/hosting.md#restricting-the-bot-to-your-chats).

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
| `ANTISPAM_BLOCK_JOB_OFFERS` | `true` | Treat job and income offers (remote work, side income, "earn $X a day") as spam when they mention pay or ask to DM. Talking about work is fine. |
| `ANTISPAM_EXEMPT_ADMINS` | `true` | Never moderate group admins. |
| `ANTISPAM_ADMIN_CACHE_TTL` | `3600` | Seconds to cache each group's admin list. |
| `ANTISPAM_ALLOWED_CHATS` | _(all chats)_ | Comma-separated chat IDs the bot is allowed to moderate. When set, every other chat is ignored. **Recommended if your bot's username is public**, so strangers can't add it and use your hosting. |
| `ANTISPAM_LEAVE_UNKNOWN_CHATS` | `false` | When `true`, the bot leaves any chat not in `ANTISPAM_ALLOWED_CHATS` instead of just ignoring it. |
| `ANTISPAM_OWNER_ID` | _(anyone)_ | Your numeric Telegram user ID. When set, only you can use `/status` in a private chat. Get it from [@userinfobot](https://t.me/userinfobot). |
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

## 🧠 How it works

```
Incoming message
   │
   ├─ 0. Anti-flood  → too many messages / same text repeated?    → delete burst + mute
   │
   ├─ 1. Heuristics  → invite links, job offers (instant, high precision) → spam → delete
   │
   ├─ 2. ML model    → spam probability ≥ threshold?              → spam → delete
   │
   └─ otherwise: allow
```

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
