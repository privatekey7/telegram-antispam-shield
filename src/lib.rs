//! Core library for the Telegram anti-spam bot.
//!
//! The crate is split into small, single-responsibility modules so the spam
//! decision logic can be unit-tested without a running Telegram bot:
//!
//! * [`features`]   — text -> hashed feature buckets (mirrors the trainer).
//! * [`model`]      — loads the linear model and scores text.
//! * [`heuristics`] — fast, high-precision regex rules.
//! * [`meta`]       — shape checks: mixed alphabets, emoji, mentions, scripts.
//! * [`text`]       — text clean-up and normalization.
//! * [`moderation`] — combines heuristics + model into a [`moderation::Verdict`].
//! * [`history`]    — recent messages and joins (repeats, campaigns, newcomers).
//! * [`trust`]      — members who earned a lighter check.
//! * [`samples`]    — messages admins marked as spam / not spam.
//! * [`warnings`]   — admin warnings that add up to a ban.
//! * [`store`]      — optional SQLite file that keeps state across restarts.
//! * [`shield`]     — the decision engine tying the above together.
//! * [`flood`]      — per-user message rate and repeat detection.
//! * [`config`]     — runtime settings read from environment variables.

pub mod config;
pub mod features;
pub mod flood;
pub mod heuristics;
pub mod history;
pub mod meta;
pub mod model;
pub mod moderation;
pub mod samples;
pub mod shield;
pub mod store;
pub mod text;
pub mod trust;
pub mod warnings;
