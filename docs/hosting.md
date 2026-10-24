# Hosting the bot

🌍 **English** · [Русский](hosting.ru.md)

The bot runs anywhere that has outbound HTTPS to `api.telegram.org`:

- **No inbound ports, no domain, no database.** It uses long polling.
- **Tiny:** about 30 MB of RAM and almost no CPU.
- **Always on:** it only moderates while it runs, so pick a machine that stays
  on (a server, a Raspberry Pi, a NAS, a PC that does not sleep).
- **One copy per token.** Telegram delivers updates to one consumer only. Stop
  any other copy of the same bot before starting a new one.

Prebuilt images for `linux/amd64` and `linux/arm64` are published at
`ghcr.io/privatekey7/telegram-antispam-shield` on every release. Tags:
`0.2.0` (exact), `0.2` (latest patch of 0.2 — recommended), `sha-<commit>`.
There is no `latest` tag on purpose.

Before you start, create the bot and get its token (README, Step 1).

> 🤖 **Let an AI agent do it.** Ask your coding agent:
> *"Install the Telegram bot from
> https://github.com/privatekey7/telegram-antispam-shield by following its
> llms-install.md."* It does everything except the few steps that need you in
> Telegram. See [llms-install.md](../llms-install.md).

## Docker Compose (recommended)

Works on any Linux server, VPS, home server, Raspberry Pi (64-bit OS) or NAS
with Docker, and on macOS/Windows with Docker Desktop.

```bash
mkdir antispam-shield && cd antispam-shield
base=https://github.com/privatekey7/telegram-antispam-shield/releases/latest/download
curl -fsSLO "$base/compose.yaml"
curl -fsSL "$base/env.example" -o .env
chmod 600 .env
nano .env                 # set BOT_TOKEN=... (and other settings if you like)
docker compose up -d
docker compose logs -f    # wait for "bot_started", then Ctrl+C
```

No Docker yet? On Debian/Ubuntu: `sudo apt-get install -y docker.io
docker-compose-v2`. Elsewhere: <https://docs.docker.com/engine/install/>.

`compose.yaml` restarts the bot after crashes and reboots and runs it hardened:
read-only filesystem, no Linux capabilities, non-root user, 128 MB memory cap,
rotated logs.

| Task | Command |
|---|---|
| Logs | `docker compose logs -f --tail 100 bot` |
| Change settings | edit `.env`, then `docker compose up -d` |
| Update (patch releases) | `docker compose pull && docker compose up -d` |
| Upgrade to a new minor/major | download the new `compose.yaml` (same command as above), read the [CHANGELOG](../CHANGELOG.md), then `docker compose up -d` |
| Pin an exact version | add `IMAGE_TAG=0.2.0` to `.env` |
| Stop | `docker compose down` |

## Plain `docker run`

Same thing in one command, if you do not want Compose:

```bash
docker run -d --name antispam-shield --restart unless-stopped \
  --env-file .env --read-only --cap-drop ALL \
  -v antispam-data:/data \
  --security-opt no-new-privileges:true --memory 128m \
  ghcr.io/privatekey7/telegram-antispam-shield:0.2
```

## Container platforms

Railway, Render, Fly.io, Koyeb, Northflank, a Kubernetes cluster — anything
that runs a Docker image as an always-on background worker:

- **Image:** `ghcr.io/privatekey7/telegram-antispam-shield:0.2`
- **Variables:** `BOT_TOKEN` and any `ANTISPAM_*` settings from the README.
- **Service type:** worker / background — no public port, no HTTP health
  check, **one** instance, restart on failure.
- **Resources:** the smallest size (≈64 MB RAM) is enough.
- **Volume (optional):** mount a small persistent volume at `/data`, so
  trusted members survive redeploys. Without it the bot works, but every
  restart puts all members back under the full checks.

Railway can also build from your fork: **New Project → Deploy from GitHub
repo**, add `BOT_TOKEN` under **Variables**. `railway.json` tells it to use the
`Dockerfile`.

Check current pricing: many platforms no longer offer a free always-on tier.

## From source (without Docker)

Needs Rust ≥ 1.88 ([rustup](https://rustup.rs)) and about 2 GB of free RAM and
disk for the build. The model is built into the binary.

```bash
cargo install --locked --tag v0.2.0 \
  --git https://github.com/privatekey7/telegram-antispam-shield
```

Run it from a folder with your `.env`:

```bash
~/.cargo/bin/telegram-antispam-shield
```

On Linux, run it as a systemd service. Copy the binary to `/usr/local/bin/`,
put the settings in `/etc/antispam-shield.env` (mode `600`) and create
`/etc/systemd/system/antispam-shield.service`:

```ini
[Unit]
Description=Telegram Anti-Spam Shield
Wants=network-online.target
After=network-online.target

[Service]
ExecStart=/usr/local/bin/telegram-antispam-shield
EnvironmentFile=/etc/antispam-shield.env
Environment=ANTISPAM_DATA_DIR=/var/lib/antispam-shield
StateDirectory=antispam-shield
DynamicUser=yes
Restart=on-failure
RestartSec=5
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
MemoryMax=128M

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now antispam-shield
journalctl -u antispam-shield -f
```

## Auto-deploy from your own fork

Changing the code and want every merge to go live on your server? See
[auto-deploy.md](auto-deploy.md).

## Restricting the bot to your chats

Every bot's username is public, so anyone can add your bot to their group,
use your server, send you spam reports and, as an admin there, feed the bot's
learned samples with `/spam`. Set `ANTISPAM_ALLOWED_CHATS` to your group
ID(s); the bot warns at startup while it is unset, and refuses to start if an
entry is not a number:

1. Stop the bot (or do this before the first start).
2. Add the bot to the group and send any message there.
3. Open `https://api.telegram.org/bot<TOKEN>/getUpdates` in a browser (keep
   this URL private — it contains your token).
4. Find `"chat":{"id":-100…` — that negative number is the group ID; `"from":
   {"id":…}` is your user ID for `ANTISPAM_OWNER_ID`.
5. Put them in `.env` and start the bot.
