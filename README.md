# 🛡 Telegram Anti-Spam Shield

🌍 **English** · [Русский](README.ru.md)

A lightweight Telegram bot that **automatically detects and deletes spam** (in
Russian and English) using a small built-in machine-learning model. It is fast,
free to run, and designed so that **anyone can deploy it in ~10 minutes** — no
coding required.

[![Deploy on Railway](https://railway.com/button.svg)](https://railway.com/new)
[![CI](https://github.com/zalupajap/telegram-antispam-shield/actions/workflows/ci.yml/badge.svg)](https://github.com/zalupajap/telegram-antispam-shield/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

---

## What it does

- 🤖 Classifies every group message as **spam** or **not spam** with a small ML model.
- 🗑 **Deletes** spam automatically (and can optionally **ban** or **mute** the sender).
- 🇷🇺 🇬🇧 Trained on real Russian + English spam.
- 🪶 Tiny footprint: runs in a few tens of MB of RAM — fits free hosting tiers.
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

## 🚀 Deploy in 10 minutes (no coding)

You will: (1) create a bot, (2) put this project on Railway, (3) add the bot to
your group. Follow the steps in order.

### Step 1 — Create your bot and get a token

1. In Telegram, open [@BotFather](https://t.me/BotFather).
2. Send `/newbot` and follow the prompts (choose a name and a username).
3. BotFather gives you a **token** that looks like
   `123456789:AAExxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx`. **Copy it** — you'll need it.
4. Send `/setprivacy` → pick your bot → choose **Disable**. This lets the bot
   see all group messages so it can check them for spam. *(Important!)*

### Step 2 — Put this bot on Railway

1. **Fork** this repository (top-right **Fork** button on GitHub).
2. Go to [railway.com](https://railway.com) and sign in with GitHub.
3. Click **New Project** → **Deploy from GitHub repo** → select your fork.
4. Railway detects the included `Dockerfile` and starts building automatically.
5. Open the **Variables** tab and add one variable:
   - Name: `BOT_TOKEN`
   - Value: the token from Step 1
6. Railway redeploys. When the **Deploy Logs** show `bot_started`, it's live. 🎉

> 💡 No credit card needed for Railway's trial/free usage. The bot uses almost
> no resources.

### Step 3 — Add the bot to your group

1. Open your Telegram **group** → **Add members** → add your bot.
2. Make the bot an **administrator** with at least the **Delete messages**
   permission. *(To also ban/mute spammers, enable **Ban users** too.)*
3. Done. The bot now removes spam automatically.

**Test it:** in the group, send something spammy like
`ЗАРАБОТОК ОТ 5000$ В ДЕНЬ! Пиши в личку` — it should be deleted within a second.

> 🧪 Want to watch before it deletes? Set `ANTISPAM_DRY_RUN=true` in Railway
> Variables. The bot will only log what it *would* delete. Set it back to
> `false` when you're confident.

---

## ⚙️ Configuration

All settings are **environment variables** (set them in Railway → Variables).
Only `BOT_TOKEN` is required; everything else has sensible defaults.

| Variable | Default | Description |
|---|---|---|
| `BOT_TOKEN` | — | **Required.** Token from @BotFather. |
| `ANTISPAM_ACTION` | `delete` | `delete`, `ban`, or `mute` the spammer. |
| `ANTISPAM_DRY_RUN` | `false` | If `true`, only log detections — never delete. |
| `ANTISPAM_THRESHOLD` | *(model default ≈ 0.82)* | Spam probability cutoff `0.0–1.0`. Higher = stricter (fewer deletions). |
| `ANTISPAM_BLOCK_INVITE_LINKS` | `true` | Treat `t.me/+…` / `joinchat` links as spam. |
| `ANTISPAM_EXEMPT_ADMINS` | `true` | Never moderate group admins. |
| `ANTISPAM_ADMIN_CACHE_TTL` | `3600` | Seconds to cache each group's admin list. |
| `RUST_LOG` | `info` | Log level: `error`/`warn`/`info`/`debug`. |
| `MODEL_PATH` | *(unset)* | Path to a custom-trained model (advanced). |

See [`.env.example`](.env.example) for a copy-paste template.

---

## 🧠 How it works

```
Incoming message
   │
   ├─ 1. Heuristics  → invite links etc. (instant, high precision) → spam → delete
   │
   ├─ 2. ML model    → spam probability ≥ threshold?              → spam → delete
   │
   └─ otherwise: allow
```

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
| Build fails on Railway | Make sure you deployed your **fork** unchanged; Railway should use the `Dockerfile`. |

---

## License

[MIT](LICENSE) — free to use, fork, and modify.
