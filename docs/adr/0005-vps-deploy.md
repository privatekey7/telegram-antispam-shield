# 5. Self-hosted VPS deploy via GHCR images

- Status: accepted, amended by [ADR 0006](0006-self-hosting-releases.md)
- Date: 2026-10-24

## Context

The reference instance ran on Railway's trial and stopped when the trial
ended. The owner already pays for a small VPS (1 vCPU, 1 GB RAM, 10 GB disk)
that runs a RustDesk relay, so hosting the bot there costs nothing extra.

Running a long-lived bot on GitHub Actions runners was considered and rejected:
it violates GitHub's terms for Actions, scheduled runs start late and leave
gaps, and in-memory anti-flood state (ADR 0004) would reset every few hours.

Building on the VPS was rejected too: a Rust release build needs several GB of
disk and has already run out of memory on small builders.

## Decision

- GitHub Actions builds the Docker image after CI passes on `main` and pushes
  it to GHCR, tagged `sha-<commit>` (immutable; no `latest`).
- The same workflow connects over SSH as an unprivileged `deploy` user whose
  key is bound to a forced command (`deploy.sh`). The script accepts only a
  `sha-<40 hex>` tag, pulls it, restarts the Compose service and rolls back to
  the previous tag if the container does not stay up.
- The container runs non-root, read-only, without capabilities, with a 128 MB
  memory cap and rotated JSON logs, so it cannot crowd out the relay.
- Deploys are opt-in per repository (`DEPLOY_ENABLED` variable), so forks of
  this public template do not try to deploy anywhere.
- Railway support (`railway.json`, README guide) stays for people who prefer a
  PaaS.

## Consequences

- Hosting cost for the reference instance drops to zero on top of the VPS.
- The deploy pipeline needs four secrets and one-time server setup
  (`docs/auto-deploy.md`).
- The `deploy` user is in the `docker` group, which is root-equivalent on the
  host; the forced command is the boundary that keeps the CI key limited.
- Rollback is a one-liner with the previous SHA.
