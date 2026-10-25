//! Combot Anti-Spam (CAS) lookups: is this user a known spammer?
//!
//! CAS (<https://cas.chat>) is a public database of accounts banned for spam
//! across many Telegram groups. Only untrusted members are looked up, results
//! are cached, and a slow or failing API counts as "not listed", so CAS can
//! only add catches, never block the bot. After a failure the API is left
//! alone for [`PAUSE_AFTER_FAILURE`], so a hanging CAS costs one timeout, not
//! one per message; answers larger than [`MAX_BODY_BYTES`] are refused.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;
use teloxide::types::UserId;

use telegram_antispam_shield::config::CasConfig;

/// Cached results kept before expired ones are dropped.
const MAX_CACHED: usize = 10_000;
/// After a failed lookup, every user counts as "not listed" for this long.
const PAUSE_AFTER_FAILURE: Duration = Duration::from_secs(60);
/// A real answer is a few hundred bytes.
const MAX_BODY_BYTES: usize = 64 * 1024;

/// The part of the `/check` response we need: `ok` is `true` when listed.
#[derive(Deserialize)]
struct CheckResponse {
    ok: bool,
}

pub struct Cas {
    config: CasConfig,
    client: reqwest::Client,
    cache: Mutex<HashMap<u64, (bool, Instant)>>,
    /// When the last lookup failed.
    failed_at: Mutex<Option<Instant>>,
}

impl Cas {
    pub fn new(config: CasConfig) -> Cas {
        Cas {
            client: reqwest::Client::builder()
                .timeout(config.timeout)
                .build()
                .unwrap_or_default(),
            config,
            cache: Mutex::new(HashMap::new()),
            failed_at: Mutex::new(None),
        }
    }

    /// Whether `user` is listed. `false` when CAS is off or unreachable.
    pub async fn is_listed(&self, user: UserId) -> bool {
        if !self.config.enabled {
            return false;
        }
        if let Some(listed) = self.cached(user) {
            return listed;
        }
        if self.paused(Instant::now()) {
            return false;
        }
        match self.lookup(user).await {
            Ok(listed) => {
                self.remember(user, listed);
                listed
            }
            Err(error) => {
                if self.pause(Instant::now()) {
                    tracing::warn!(%error, pause_secs = PAUSE_AFTER_FAILURE.as_secs(), "cas_paused");
                }
                false
            }
        }
    }

    async fn lookup(&self, user: UserId) -> Result<bool, String> {
        let url = format!("{}/check", self.config.url.trim_end_matches('/'));
        let mut response = self
            .client
            .get(url)
            .query(&[("user_id", user.0)])
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| e.to_string())?;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            if body.len() + chunk.len() > MAX_BODY_BYTES {
                return Err(format!("answer over {MAX_BODY_BYTES} bytes"));
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice::<CheckResponse>(&body)
            .map(|r| r.ok)
            .map_err(|e| e.to_string())
    }

    /// Whether a recent failure says to leave the API alone.
    fn paused(&self, now: Instant) -> bool {
        let failed_at = self.failed_at.lock().unwrap_or_else(|e| e.into_inner());
        failed_at.is_some_and(|at| now.duration_since(at) < PAUSE_AFTER_FAILURE)
    }

    /// Start a pause; `true` if none was running (worth a log line).
    fn pause(&self, now: Instant) -> bool {
        let mut failed_at = self.failed_at.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = failed_at.is_none_or(|at| now.duration_since(at) >= PAUSE_AFTER_FAILURE);
        *failed_at = Some(now);
        fresh
    }

    fn cached(&self, user: UserId) -> Option<bool> {
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .get(&user.0)
            .filter(|(_, at)| at.elapsed() < self.config.cache_ttl)
            .map(|(listed, _)| *listed)
    }

    fn remember(&self, user: UserId, listed: bool) {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= MAX_CACHED {
            let ttl = self.config.cache_ttl;
            cache.retain(|_, (_, at)| at.elapsed() < ttl);
        }
        if cache.len() >= MAX_CACHED {
            // Still full of fresh results (a raid of new accounts): start
            // over rather than grow without bound. Lookups are only repeated.
            cache.clear();
        }
        cache.insert(user.0, (listed, Instant::now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listed_and_unlisted_responses_parse() {
        let listed = r#"{"ok":true,"result":{"offenses":3,"messages":["spam"],"time_added":"2024-01-01T00:00:00.000Z"}}"#;
        let unlisted = r#"{"ok":false,"description":"Record not found."}"#;

        assert!(serde_json::from_str::<CheckResponse>(listed).unwrap().ok);
        assert!(!serde_json::from_str::<CheckResponse>(unlisted).unwrap().ok);
    }

    #[test]
    fn cache_never_grows_past_its_cap() {
        let cas = Cas::new(CasConfig::default());
        for user in 0..(MAX_CACHED as u64 + 10) {
            cas.remember(UserId(user), false);
        }
        let cached = cas.cache.lock().unwrap().len();
        assert!(cached <= MAX_CACHED, "{cached} cached");
    }

    #[test]
    fn a_failure_pauses_lookups_for_a_while() {
        // Arrange
        let cas = Cas::new(CasConfig::default());
        let now = Instant::now();

        // Act
        let first = cas.pause(now);
        let again = cas.pause(now + Duration::from_secs(1));

        // Assert
        assert!(first);
        assert!(!again, "one log line per pause");
        assert!(cas.paused(now + Duration::from_secs(30)));
        assert!(!cas.paused(now + Duration::from_secs(1) + PAUSE_AFTER_FAILURE));
    }

    #[tokio::test]
    async fn an_unreachable_cas_is_asked_once_per_pause() {
        // Arrange: nothing listens on port 9.
        let cas = Cas::new(CasConfig {
            url: "http://127.0.0.1:9".to_string(),
            timeout: Duration::from_secs(2),
            ..CasConfig::default()
        });

        // Act
        let first = cas.is_listed(UserId(1)).await;
        let paused_now = cas.paused(Instant::now());

        // Assert
        assert!(!first);
        assert!(paused_now);
    }

    #[tokio::test]
    async fn disabled_cas_never_calls_out() {
        let cas = Cas::new(CasConfig {
            enabled: false,
            url: "http://127.0.0.1:9".to_string(),
            ..CasConfig::default()
        });
        assert!(!cas.is_listed(UserId(1)).await);
    }
}
