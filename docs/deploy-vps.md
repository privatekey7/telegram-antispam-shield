# Deploying to your own VPS

Runbook for running the bot on a small Linux server (tested target: Ubuntu
26.04, 1 vCPU / 1 GB RAM, shared with other services). GitHub builds the image
and deploys it; the server only pulls and runs it.

```
push to main → CI green → Deploy workflow
                            ├─ build image → ghcr.io/<owner>/telegram-antispam-shield:sha-<commit>
                            └─ ssh deploy@server "sha-<commit>"   (forced command → deploy.sh)
                                   └─ docker compose pull && up -d, roll back if the bot does not stay up
```

Why this shape (details in [ADR 0005](adr/0005-vps-deploy.md)):

- **No builds on the server.** Compiling Rust needs gigabytes of disk and RAM;
  the runtime image is ~100 MB and the bot uses ~30 MB of RAM.
- **Immutable tags.** Every image is tagged by commit SHA, never `latest`, so a
  rollback is "deploy the previous SHA".
- **Least privilege.** CI holds an SSH key that can only run `deploy.sh` with a
  strictly validated tag. The container runs as non-root with a read-only
  filesystem, no capabilities and a 128 MB memory cap.
- **One instance only.** Telegram delivers updates to a single long-polling
  consumer per token. Stop any other deployment (e.g. Railway) of the same bot.

## 1. Prepare the server (once, as root)

Check that there is room:

```bash
free -h && df -h / && docker --version; docker ps
```

Install Docker if it is missing (Ubuntu packages are enough):

```bash
apt-get update && apt-get install -y docker.io docker-compose-v2
systemctl enable --now docker
```

Create an unprivileged deploy user and the app directory:

```bash
useradd --create-home --shell /bin/sh deploy
usermod -aG docker deploy
install -d -o deploy -g deploy -m 750 /opt/antispam-shield
cd /opt/antispam-shield
base=https://raw.githubusercontent.com/privatekey7/telegram-antispam-shield/main/deploy
curl -fsSLO "$base/compose.yaml" && curl -fsSLO "$base/deploy.sh"
chmod 755 deploy.sh
```

Create `bot.env` with the bot settings (same variables as `.env.example`) and
lock it down:

```bash
install -o deploy -g deploy -m 600 /dev/null bot.env
nano bot.env          # BOT_TOKEN=..., ANTISPAM_ALLOWED_CHATS=..., ANTISPAM_OWNER_ID=...
chown deploy:deploy compose.yaml deploy.sh
```

## 2. Give GitHub a deploy key

On your own machine, create a key pair used only for deploys:

```bash
ssh-keygen -t ed25519 -N "" -C github-deploy -f antispam_deploy_key
```

On the server, authorize the **public** key for `deploy` with a forced command,
so the key can do nothing except run `deploy.sh`:

```bash
install -d -o deploy -g deploy -m 700 /home/deploy/.ssh
echo 'command="/opt/antispam-shield/deploy.sh",no-port-forwarding,no-X11-forwarding,no-agent-forwarding,no-pty <contents of antispam_deploy_key.pub>' \
  >> /home/deploy/.ssh/authorized_keys
chown deploy:deploy /home/deploy/.ssh/authorized_keys
chmod 600 /home/deploy/.ssh/authorized_keys
```

Note: membership in the `docker` group is root-equivalent on that host. The
forced command is what keeps the CI key limited to deploys.

## 3. Configure the repository

In **Settings → Environments**, create an environment `production` with these
secrets:

| Secret | Value |
|---|---|
| `DEPLOY_HOST` | server IP or hostname |
| `DEPLOY_USER` | `deploy` |
| `DEPLOY_SSH_KEY` | contents of `antispam_deploy_key` (the private key) |
| `DEPLOY_KNOWN_HOSTS` | output of `ssh-keyscan -t ed25519 <server>` run from a trusted machine |

In **Settings → Secrets and variables → Actions → Variables**, add
`DEPLOY_ENABLED` = `true`. Without it the Deploy workflow does nothing, which
keeps forks inert.

Then delete the local private key file.

## 4. First deploy

Run **Actions → Deploy → Run workflow**. The first push creates the package
`ghcr.io/<owner>/telegram-antispam-shield`. If the deploy step fails with
`denied` / `unauthorized` while pulling, open the package settings on GitHub
and set its visibility to **Public** (the source is public anyway), then re-run.

After that, every merge to `main` with green CI deploys automatically.

## Operations

All commands run on the server from `/opt/antispam-shield`.

| Task | Command |
|---|---|
| Logs | `docker compose logs -f --tail 100 bot` |
| Status | `docker compose ps` |
| Restart | `docker compose restart bot` |
| Change settings | edit `bot.env`, then `docker compose up -d` |
| Roll back | `./deploy.sh sha-<previous commit>` (previous tag is in `.env.previous`) |
| Stop | `docker compose down` |

### Troubleshooting

| Symptom | Likely cause → fix |
|---|---|
| `TerminatedByOtherGetUpdates` / conflict in logs | Another instance uses the same token → stop it (Railway, local `cargo run`). |
| `delete_failed` / `flood_delete_failed` | Bot is not a group admin with **Delete messages**. |
| `flood_mute_failed` / `mute_failed` | Bot lacks **Ban users**, or the chat is a basic group (mutes need a supergroup). |
| Deploy job: `Permission denied (publickey)` | Public key missing or malformed in `authorized_keys`, or wrong `DEPLOY_USER`. |
| Deploy job: `Host key verification failed` | `DEPLOY_KNOWN_HOSTS` does not match the server; re-run `ssh-keyscan`. |
| `deploy.sh` rolled back | The new container exited; its last 50 log lines are in the job output. |
