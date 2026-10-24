# Auto-deploy from your fork (advanced)

Most people do not need this: install a release with
[docs/hosting.md](hosting.md) and update it with `docker compose pull`.

This runbook is for people who change the code in their own fork and want
every merge to `main` to go live on their own Linux server automatically.
GitHub builds the image and deploys it; the server only pulls and runs it.

```
push to main → CI green → Deploy workflow
                            ├─ build image → ghcr.io/<you>/telegram-antispam-shield:sha-<commit>
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
  consumer per token. Stop any other copy of the same bot first.

In the commands below, replace `<you>` with your GitHub user or organisation
(lowercase).

## 1. Prepare the server (once, as root)

Any 64-bit Linux server with Docker works (amd64 or arm64). Check that there
is room:

```bash
free -h && df -h / && docker --version; docker ps
```

Install Docker if it is missing. On Debian/Ubuntu the distribution packages are
enough:

```bash
apt-get update && apt-get install -y docker.io docker-compose-v2
systemctl enable --now docker
```

Other distributions: see <https://docs.docker.com/engine/install/>.

Create an unprivileged deploy user and the app directory:

```bash
useradd --create-home --shell /bin/sh deploy
usermod -aG docker deploy
install -d -o deploy -g deploy -m 750 /opt/antispam-shield
cd /opt/antispam-shield
base=https://raw.githubusercontent.com/<you>/telegram-antispam-shield/main
curl -fsSLO "$base/compose.yaml" && curl -fsSLO "$base/deploy/deploy.sh"
curl -fsSL "$base/.env.example" -o .env
chmod 755 deploy.sh
chmod 600 .env
chown deploy:deploy compose.yaml deploy.sh .env
```

Edit `.env`: set `BOT_TOKEN` and any other settings, and point Compose at your
own image:

```bash
nano .env      # BOT_TOKEN=..., ANTISPAM_ALLOWED_CHATS=..., ANTISPAM_OWNER_ID=...
echo 'IMAGE=ghcr.io/<you>/telegram-antispam-shield' >> .env
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

Protect the environment in the same screen. These secrets open a shell on
your server, and the server holds `BOT_TOKEN`. (GitHub offers these rules on
public repositories; private ones need a paid plan.)

- **Deployment branches and tags** → *Selected branches and tags* → add
  `main`. This blocks manual runs started from any other branch. It does
  **not** block CI-triggered runs: a `workflow_run` workflow always runs on
  the default branch, so GitHub sees `main` whatever started it.
- **Required reviewers** → add yourself. Every deploy then waits until you
  click **Approve** in the run. This is the only gate that still holds if the
  check in the workflow is ever broken, for example by an edit in your fork.
  The price: deploys are no longer fully automatic. If you are the only
  maintainer, leave *Prevent self-review* off, or nobody can approve.

In **Settings → Secrets and variables → Actions → Variables**, add
`DEPLOY_ENABLED` = `true`. Without it the Deploy workflow does nothing, which
keeps forks inert.

The bot token is not a GitHub secret: it stays in `.env` on the server.

Then delete the local private key file.

## 4. First deploy

Run **Actions → Deploy → Run workflow** on branch `main` (runs from other
branches are skipped). If you added required reviewers, approve the
`production` deployment in the run. The first push creates the package
`ghcr.io/<you>/telegram-antispam-shield`. If the deploy step fails with
`denied` / `unauthorized` while pulling, open the package settings on GitHub
and set its visibility to **Public** (the source is public anyway), then re-run.

After that, every merge to `main` with green CI deploys automatically.

## Who can trigger a deploy

Only a push to `main` of your repository, or a manual run from `main`, by
someone with write access. Pull requests never deploy, whether they come from
a fork or not.

That takes an explicit check. Deploy starts when CI completes, and its
`branches: [main]` filter compares only the branch *name*. A pull request
from a fork whose branch is called `main` passes that filter and runs CI in
your repository. Without the check, its code would be built and deployed to
your server. So the `image` job in `.github/workflows/deploy.yml` runs only
when:

- `DEPLOY_ENABLED` is `true`, and
- it is a manual run (`workflow_dispatch`) from `refs/heads/main`, or a CI run
  that succeeded and that was started by a `push` (not `pull_request`) to the
  branch `main` of this repository (`head_repository.full_name ==
  github.repository`).

After checkout the job also checks that the commit is in the history of
`origin/main`, and it tags the image from that commit only. `deploy` runs
only after `image` succeeds and ships the tag that `image` built. Pull
request runs also stay out of the deploy concurrency group, so they cannot
cancel a queued deploy.

If you edit `deploy.yml`, keep these checks. Required reviewers on
`production` are the backstop if a check breaks.

## Operations

All commands run on the server from `/opt/antispam-shield`.

| Task | Command |
|---|---|
| Logs | `docker compose logs -f --tail 100 bot` |
| Status | `docker compose ps` |
| Restart | `docker compose restart bot` |
| Change settings | edit `.env`, then `docker compose up -d` |
| Roll back | `./deploy.sh sha-<previous commit>` (previous tag is in `.env.previous`) |
| Stop | `docker compose down` |

### Troubleshooting

| Symptom | Likely cause → fix |
|---|---|
| `TerminatedByOtherGetUpdates` / conflict in logs | Another copy uses the same token (another server, a PaaS, a local `cargo run`) → stop it. |
| `delete_failed` / `flood_delete_failed` | Bot is not a group admin with **Delete messages**. |
| `flood_mute_failed` / `mute_failed` | Bot lacks **Ban users**, or the chat is a basic group (mutes need a supergroup). |
| Deploy job: `Permission denied (publickey)` | Public key missing or malformed in `authorized_keys`, or wrong `DEPLOY_USER`. |
| Deploy job: `Host key verification failed` | `DEPLOY_KNOWN_HOSTS` does not match the server; re-run `ssh-keyscan`. |
| `deploy.sh` rolled back | The new container exited; its last 50 log lines are in the job output. |
