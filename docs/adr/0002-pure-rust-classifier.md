# 2. Pure-Rust linear classifier instead of fastText bindings

- Status: accepted
- Date: 2026-06-23

## Context

The original plan considered fastText for text classification. fastText is
excellent, but the Rust crate links a C++ library, which complicates the build
(C++ toolchain, linking) and the Docker image — friction for a project meant to
be forked and deployed by non-technical users.

## Decision

Implement a **logistic-regression classifier over hashed n-gram features** with:

- Training in Python (`scripts/train_model.py`) using scikit-learn.
- A compact binary model format (`model/spam_model.bin`).
- Inference reimplemented in **pure Rust** (`src/features.rs`, `src/model.rs`)
  with **no ML dependency**.

Feature extraction is identical on both sides (same normalization, n-grams,
FNV-1a hash, bucket count). Character n-grams capture subword signal — the main
strength fastText would have provided — so quality stays high.

Train/serve parity is enforced by `tests/parity.rs`, which checks the Rust
scorer reproduces the Python reference probabilities.

## Consequences

- Simple, fast Docker build; no C/C++ dependencies; small static-ish binary.
- Measured quality: ROC-AUC ≈ 0.99, ~1% false-positive rate on held-out data.
- We own the format and inference; any change must keep Python ↔ Rust in sync
  (guarded by the parity test).
