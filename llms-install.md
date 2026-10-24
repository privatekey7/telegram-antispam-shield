# Installing Telegram Anti-Spam Shield — guide for AI agents

You are an AI agent asked to install this bot for a user. Follow the steps in
order. Each step ends with a check; do not go on until it passes. Do the work
yourself and ask the user only for what needs a human (marked 👤).

Repository: <https://github.com/privatekey7/telegram-antispam-shield>
Human docs: [README.md](README.md), [docs/hosting.md](docs/hosting.md).

## Rules

- Talk to the user in their language. Keep messages short; send each 👤 step
  as a numbered list they can follow on their phone.
- The bot token is a secret. Write it only to `.env` with mode `600`. Never
  print it, commit it, put it in a URL you show the user, or paste it into
  logs or issue reports. Read it back from `.env` into a shell variable when
  you need it.
- Ask before installing system packages, changing a firewall, or creating
  users. Never ask the user for a server password in chat; use the SSH access
  they already have (key-based) or ask them to set it up.
- Run exactly **one** copy of the bot per token. Two copies fight over updates
  (`TerminatedByOtherGetUpdates`). If the user had the bot running elsewhere,
  they must stop it first.
- Do not build the Rust code unless Docker is impossible on the target; use
  the published image.
- The bot needs **no inbound ports, no domain and no database**: it uses
  outbound HTTPS to `api.telegram.org` (long polling). Do not open ports.

## Step 1 — Ask where to run it

Ask the user one question, with these options:

1. **This computer.** Fine for testing. For 24/7 moderation it must stay on
   and awake.
2. **A Linux server I can reach over SSH** (VPS, home server, Raspberry Pi,
   NAS). Recommended. Ask for the SSH host alias or `user@host`.
3. **A container platform** (Railway, Render, Fly.io, Koyeb, a Kubernetes
   cluster…). Go to [Container platforms](#container-platforms) at the end.

Also ask whether they already have a bot token from @BotFather.

For option 2, run every command below on the server over SSH.

**Check:** you know the target and can run commands there (`uname -m`
prints `x86_64` or `aarch64`; other architectures are not published).

## Step 2 — Docker

```bash
docker --version && docker compose version
```

If both work, go on. If not:

- Debian/Ubuntu: ask, then
  `sudo apt-get update && sudo apt-get install -y docker.io docker-compose-v2 && sudo systemctl enable --now docker`.
- Other Linux: follow <https://docs.docker.com/engine/install/> for the distro.
- macOS/Windows: the user installs Docker Desktop themselves (GUI installer).
- No Docker possible: use [From source](#from-source-no-docker) instead.

If `docker` needs `sudo` for this user, either prefix the commands with `sudo`
or, with the user's consent, add them to the `docker` group (root-equivalent).

**Check:** `docker run --rm hello-world` succeeds.

## Step 3 — 👤 Create the bot (skip if they have a token)

Send the user these steps:

1. Open [@BotFather](https://t.me/BotFather) in Telegram.
2. Send `/newbot`, choose a name and a username ending in `bot`.
3. Send `/setprivacy` → choose the new bot → **Disable**. (The bot must see
   all group messages to check them.)
4. Copy the token (looks like `123456789:AA…`) and send it to me — or, if you
   prefer, paste it into `.env` yourself when I create the file.

## Step 4 — Download the files

```bash
mkdir -p ~/antispam-shield && cd ~/antispam-shield
base=https://github.com/privatekey7/telegram-antispam-shield/releases/latest/download
curl -fsSLO "$base/compose.yaml"
curl -fsSL "$base/env.example" -o .env
chmod 600 .env
```

`compose.yaml` already pins the matching image version and hardens the
container (read-only, no capabilities, 128 MB memory cap, rotated logs).

**Check:** both files exist and `.env` is `-rw-------`.

## Step 5 — Token

Set `BOT_TOKEN=` in `.env` (replace the placeholder line; do not add a second
one). Then verify it without printing it:

```bash
TOKEN="$(sed -n 's/^BOT_TOKEN=//p' .env)"
curl -fsS "https://api.telegram.org/bot$TOKEN/getMe"
```

**Check:** the response has `"ok":true`. Note `result.id` (the bot's user ID)
and `result.username`.

- `"can_read_all_group_messages":false` → privacy mode is still on. Ask the
  user to repeat `/setprivacy` → **Disable** in @BotFather, then re-check.
- HTTP 401 → wrong token; ask for it again.

## Step 6 — 👤 Add the bot to the group

Send the user these steps:

1. Open the group → **Add members** → add `@<bot username>`.
2. Open the group settings → **Administrators** → add the bot with
   **Delete messages** and **Ban users** (the second one lets it mute
   flooders).
3. Send any message in the group (for example "hi").

## Step 7 — Find the chat and the owner, lock the bot to them

The bot is not running yet, so you can read its pending updates directly:

```bash
curl -fsS "https://api.telegram.org/bot$TOKEN/getUpdates"
```

If this returns a `Conflict` error, another copy of the bot is running or a
webhook is set: `getWebhookInfo` shows it. Ask the user before deleting a
webhook (`deleteWebhook`) — it may belong to another deployment they still
need.

From the latest `my_chat_member` or `message` update for the group, take:

- `chat.id` — a negative number such as `-1001234567890`;
- `chat.type` — should be `supergroup`;
- `from.id` — the user who added the bot or wrote the message (the owner).

If several groups appear, confirm the right one with the user by its
`chat.title`. If an update has `migrate_to_chat_id`, the group was upgraded to
a supergroup — use the new ID.

Write them to `.env` (uncomment and replace the example lines):

```
ANTISPAM_ALLOWED_CHATS=<chat.id>
ANTISPAM_OWNER_ID=<from.id>
```

The allowlist stops strangers from adding the bot to their chats and using
the user's server. For several groups, separate IDs with commas.

## Step 8 — Check the bot's rights

```bash
curl -fsS "https://api.telegram.org/bot$TOKEN/getChatMember?chat_id=<chat.id>&user_id=<bot id>"
```

**Check:** `"status":"administrator"`, `"can_delete_messages":true` and
`"can_restrict_members":true`. If a right is missing, ask the user to enable it
in the group's admin settings and re-check. In a basic group (`chat.type` is
`group`) mutes do not work; deletion still does.

## Step 9 — Start

```bash
docker compose up -d
sleep 10
docker compose ps
docker compose logs --tail 50 bot
```

**Check:** the container is `running` and the logs contain `model_loaded`
(with `persistent=true`) and `bot_started`, with no `ERROR` lines.

| Log line | Fix |
|---|---|
| `TerminatedByOtherGetUpdates` / conflict | Another copy uses this token. Stop it. |
| `BOT_TOKEN is required` / `Unauthorized`, container restarting | Fix `BOT_TOKEN` in `.env`, then `docker compose up -d`. |
| `store_unavailable` | The bot runs, but trusted members reset on restart. Check that `compose.yaml` mounts the `data` volume at `/data`. |
| `pull access denied` / `manifest unknown` | Check internet access to `ghcr.io`; check `uname -m` is supported. |

## Step 10 — 👤 Smoke test

Ask the user to send this from a **non-admin** account (admins are never
moderated):

```
ЗАРАБОТОК ОТ 5000$ В ДЕНЬ! Пиши в личку
```

**Check:** within a few seconds the message disappears and the logs show
`spam_detected`. If the logs show `delete_failed`, the bot lacks **Delete
messages**.

Optional: to watch without deleting anything first, set
`ANTISPAM_DRY_RUN=true` in `.env` and run `docker compose up -d`; switch it
back to `false` afterwards.

## Step 11 — Report to the user

Tell the user, briefly:

- where it runs (host and folder `~/antispam-shield`);
- that it restarts by itself after reboots (`restart: unless-stopped`; Docker
  must start on boot — `systemctl is-enabled docker`);
- the commands they may need:

| Task | Command (in `~/antispam-shield`) |
|---|---|
| Logs | `docker compose logs -f --tail 100 bot` |
| Change settings | edit `.env`, then `docker compose up -d` |
| Update to the latest patch | `docker compose pull && docker compose up -d` |
| Stop | `docker compose down` |

All settings are listed in the README "Configuration" table and in `.env`.

---

## Container platforms

Any platform that runs a Docker image as an always-on **background worker**
works. Set:

- **Image:** `ghcr.io/privatekey7/telegram-antispam-shield:0.2`
- **Variables:** `BOT_TOKEN`, plus `ANTISPAM_ALLOWED_CHATS` and
  `ANTISPAM_OWNER_ID` (find them with steps 5–8; run those `curl` commands on
  any machine).
- **No public port / no HTTP service, one instance, restart on failure.**
  If the platform insists on a health-check port, choose its "worker" or
  "background" service type instead.
- About 64 MB RAM and a small CPU share are enough.

Railway can also build from a fork: `railway.json` in the repository tells it
to use the `Dockerfile`.

Check pricing with the user: many platforms no longer have a free tier for
always-on workers.

## From source (no Docker)

Needs Rust ≥ 1.88 (`rustup`) and ~2 GB of free RAM/disk for the build.

```bash
cargo install --locked --git https://github.com/privatekey7/telegram-antispam-shield \
  --tag "$(curl -fsSL https://api.github.com/repos/privatekey7/telegram-antispam-shield/releases/latest | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p')"
```

The binary is `~/.cargo/bin/telegram-antispam-shield`; the model is built
into it. It reads settings from environment variables or from `.env` in the
working directory. To run it as a service on Linux, see the systemd example
in [docs/hosting.md](docs/hosting.md#from-source-without-docker).
