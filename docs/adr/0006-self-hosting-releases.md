# 6. Self-hosting first: versioned release images

- Status: accepted
- Date: 2026-10-24

## Context

The project is meant to be installed by anyone from GitHub, not only by its
author. Until now every path assumed a particular host: the README walked
people through Railway (whose free tier is now a time-limited trial), and the
only published images were the `sha-<commit>` builds of one person's
auto-deploy (ADR 0005). Someone without a fork and a CI pipeline had to build
the Rust image themselves.

People also increasingly ask an AI coding agent to set things up for them, and
an agent needs instructions it can follow step by step and verify.

## Decision

- Pushing a `vX.Y.Z` tag runs `release.yml`, which builds the image natively
  for `linux/amd64` and `linux/arm64` (GitHub's ARM runners; QEMU is too slow
  for a Rust build), publishes one multi-arch manifest to GHCR and creates a
  GitHub Release with the notes from `CHANGELOG.md`.
- Image tags are `X.Y.Z`, `X.Y`, `X` (from 1.0 on) and `sha-<commit>`. There is
  no `latest`: users pin a version line such as `0.2` and still get patch
  releases, but never a breaking upgrade by surprise.
- A root `compose.yaml` is the one way to run the bot with Docker. It defaults
  to the current release line and reads every setting from `.env`. Releases
  attach it (and `env.example`), so `releases/latest/download/...` always gives
  a matching pair. The release job refuses to publish if `Cargo.toml`,
  `compose.yaml` and `CHANGELOG.md` disagree with the tag.
- Documentation is host-neutral: `docs/hosting.md` covers any Docker host
  (server, home PC, Raspberry Pi, NAS), container platforms and a from-source
  install. `llms-install.md` is a step-by-step guide for AI agents with
  checks after each step, using the Telegram Bot API to verify the token,
  privacy mode, chat ID and admin rights so the user only does what needs a
  human (BotFather, adding the bot to the group).
- The SSH auto-deploy from ADR 0005 stays as an optional, advanced path for
  people who run their own fork (`docs/auto-deploy.md`) and now uses the same
  `compose.yaml` and `.env`.

## Consequences

- Installing needs only Docker and a bot token; no Rust toolchain, no fork.
- Every release must bump `Cargo.toml`, the `IMAGE_TAG` default in
  `compose.yaml` (on a minor or major bump) and `CHANGELOG.md` together; CI
  enforces it.
- The GHCR package must be public for anonymous pulls (a one-time setting).
- Secrets never touch the repository or the image: each installation keeps its
  own token in its own `.env` or in its platform's variables.
