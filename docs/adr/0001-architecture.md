# 1. Architecture: cascade of heuristics + small ML model

- Status: accepted
- Date: 2026-06-23

## Context

We need a Telegram anti-spam bot that detects and deletes spam (Russian +
English) while running on a free hosting tier (~512 MB RAM). It must be easy for
non-technical admins to fork and deploy.

## Decision

Use a two-stage cascade:

1. **Heuristics** — fast, high-precision regex rules (e.g. Telegram invite
   links). They handle obvious spam instantly and never touch the model.
2. **ML model** — a small linear classifier for the "grey zone".

The model is **embedded into the binary** (`include_bytes!`) so deployment needs
no extra files or volumes. Threshold is tuned for a low false-positive rate:
deleting a legitimate message is worse than missing one spam message.

## Consequences

- Tiny runtime footprint (~tens of MB RAM), fits free tiers comfortably.
- No external API calls or per-message cost; works fully offline.
- Adding rules or retraining the model are independent, low-risk changes.
