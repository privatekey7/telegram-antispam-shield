//! Core library for the Telegram anti-spam bot.
//!
//! The crate is split into small, single-responsibility modules so the spam
//! decision logic can be unit-tested without a running Telegram bot:
//!
//! * [`features`]   — text -> hashed feature buckets (mirrors the trainer).
//! * [`model`]      — loads the linear model and scores text.
//! * [`heuristics`] — fast, high-precision regex rules.
//! * [`moderation`] — combines heuristics + model into a [`moderation::Verdict`].
//! * [`config`]     — runtime settings read from environment variables.

pub mod config;
pub mod features;
pub mod heuristics;
pub mod model;
pub mod moderation;
