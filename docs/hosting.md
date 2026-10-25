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
`0.3.0` (exact), `0.3` (latest patch of 0.3 — recommended), `sha-<commit>`.
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

The bot starts with no chat allowed. Add it to your group and allow the group
as described in [Restricting the bot to your chats](#restricting-the-bot-to-your-chats).

No Docker yet? On Debian/Ubuntu: `sudo apt-get install -y docker.io
docker-compose-v2`. Elsewhere: <https://docs.docker.com/engine/install/>.

`compose.yaml` restarts the bot after crashes and reboots and runs it hardened:
read-only filesystem, no Linux capabilities, non-root user, 128 MB memory and
128-process caps, rotated logs. The image is distroless: no shell and no
package manager.

| Task | Command |
|---|---|
| Logs | `docker compose logs -f --tail 100 bot` |
| Change settings | edit `.env`, then `docker compose up -d` |
| Update (patch releases) | `docker compose pull && docker compose up -d` |
| Upgrade to a new minor/major | download the new `compose.yaml` (same command as above), read the [CHANGELOG](../CHANGELOG.md), then `docker compose up -d` |
| Pin an exact version | add `IMAGE_TAG=0.3.0` to `.env` |
| Stop | `docker compose down` |

## Plain `docker run`

Same thing in one command, if you do not want Compose:

```bash
docker run -d --name antispam-shield --restart unless-stopped \
  --env-file .env --read-only --tmpfs /tmp:size=1m --cap-drop ALL \
  -v antispam-data:/data \
  --security-opt no-new-privileges:true --memory 128m --cpus 0.5 \
  --pids-limit 128 --log-opt max-size=10m --log-opt max-file=3 \
  ghcr.io/privatekey7/telegram-antispam-shield:0.3
```

`--tmpfs /tmp` is needed for the health check (see [Health](#health)); keep
the same `-v antispam-data:/data` on every run, or a new anonymous volume
starts the bot from scratch.

## Container platforms

Railway, Render, Fly.io, Koyeb, Northflank, a Kubernetes cluster — anything
that runs a Docker image as an always-on background worker:

- **Image:** `ghcr.io/privatekey7/telegram-antispam-shield:0.3`
- **Variables:** `BOT_TOKEN`, `ANTISPAM_ALLOWED_CHATS` and any other
  `ANTISPAM_*` settings from the README.
- **Service type:** worker / background — no public port, no HTTP health
  check, **one** instance, restart on failure.
- **Resources:** the smallest size (≈64 MB RAM) is enough.
- **Volume (recommended):** mount a small persistent volume at `/data`, so
  trusted members, learned samples, warnings and reports survive redeploys.
  Without it the bot works, but every restart forgets all of them, and report
  buttons from before the restart stop working.

Railway can also build from your fork: **New Project → Deploy from GitHub
repo**, add `BOT_TOKEN` under **Variables**. `railway.json` tells it to use the
`Dockerfile`.

Check current pricing: many platforms no longer offer a free always-on tier.

## From source (without Docker)

Needs Rust ≥ 1.88 ([rustup](https://rustup.rs)) and about 2 GB of free RAM and
disk for the build. The model is built into the binary.

```bash
cargo install --locked --tag v0.3.0 \
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

Every bot's username is public, so anyone can add your bot to their group.
So the bot moderates only the groups in `ANTISPAM_ALLOWED_CHATS`: anywhere
else it reads nothing, sends no reports and learns nothing. Until the list is
set it moderates no chat at all and says so at startup. An entry that is not
a number stops the bot.

**From the log** (the bot is running):

1. Add the bot to the group and send any message there.
2. `docker compose logs bot | grep chat_not_allowed` shows
   `chat_not_allowed … chat=-100…`. That negative number is the group ID.
3. Put it in `.env` (`ANTISPAM_ALLOWED_CHATS=-100…`, several separated by
   commas) and run `docker compose up -d`.

**From Telegram** (the bot is stopped, or not started yet):

1. Add the bot to the group and send any message there.
2. In the folder with `.env`, fetch the pending updates. The token is read
   from `.env` and passed to curl on stdin, so it stays out of browser
   history, the shell history and the process list:

   ```bash
   printf 'url = "https://api.telegram.org/bot%s/getUpdates"\n' \
     "$(sed -n 's/^BOT_TOKEN=//p' .env)" | curl -fsS -K -
   ```
3. Find `"chat":{"id":-100…` — that negative number is the group ID; `"from":
   {"id":…}` is your user ID for `ANTISPAM_OWNER_ID`.
4. Put them in `.env` and start the bot.

`ANTISPAM_ALLOW_ALL_CHATS=true` makes the bot moderate every group it is
added to, as it did before 0.3. Strangers can then use your server, send you
reports (at most 20 per chat per 10 minutes, from any number of chats) and
fill the shared budget of learned samples, so a chat that joins later may
keep only a few.

## Health

The image has a `HEALTHCHECK`: the bot writes a heartbeat to
`/tmp/antispam-heartbeat` after every successful call to Telegram, and
`telegram-antispam-shield healthcheck` fails once it is older than 90 s.
`docker compose ps` shows `healthy` or `unhealthy`, and
`docker inspect -f '{{.State.Health.Status}}' <container>` prints the status
alone. With `read_only: true`, `/tmp` must be writable; compose.yaml mounts a
small tmpfs there. Set `ANTISPAM_HEARTBEAT_FILE` to move the file.

## What the database holds

`antispam.db` in the data volume keeps, per chat:

- **members**: user IDs with their count of clean messages and whether they
  are trusted. Untrusted members silent for 30 days are deleted, trusted ones
  once their trust lapses (`ANTISPAM_TRUST_TTL_DAYS`).
- **warnings**: user IDs with their `/warn` count, deleted a month after the
  last warning.
- **samples**: texts admins marked as spam or not spam, stored as hashed word
  lists, at most 1,000 per kind and chat.
- **reports**: the last 1,000 spam reports, with the chat, the sender's ID and
  the first 800 characters of the text, for the report buttons.
- **samples_unscoped**: samples from before 0.2.1, never used (see
  [ADR 0011](adr/0011-per-chat-bounded-samples.md)). Drop it once you no
  longer need it: `DROP TABLE samples_unscoped;`.

Nothing else is stored: no message history and no names. Back the file up
before upgrades (CHANGELOG says when the schema changes).

## Verifying the image

Release images from 0.2.1 on are signed with
[cosign](https://docs.sigstore.dev/) (keyless, by the Release workflow) and
carry an SPDX SBOM and SLSA provenance. 0.2.0 and older are not signed.

```bash
image=ghcr.io/privatekey7/telegram-antispam-shield:<version>
cosign verify "$image" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --certificate-identity-regexp '^https://github\.com/privatekey7/telegram-antispam-shield/\.github/workflows/release\.yml@refs/tags/v[0-9]+\.[0-9]+\.[0-9]+$'
docker buildx imagetools inspect "$image" --format '{{ json .SBOM }}'
docker buildx imagetools inspect "$image" --format '{{ json .Provenance }}'
```

The Release workflow publishes only tags whose commit is on `main` and passed
CI.

To run exactly the image you verified, put its digest in `.env`
(`IMAGE_TAG=<version>@sha256:…`, the digest from `cosign verify`). A tag
can be moved; a digest cannot.
