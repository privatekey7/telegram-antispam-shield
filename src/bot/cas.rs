//! Combot Anti-Spam (CAS) lookups: is this user a known spammer?
//!
//! CAS (<https://cas.chat>) is a public database of accounts banned for spam
//! across many Telegram groups. Only untrusted members are looked up, results
//! are cached, and a slow or failing API counts as "not listed", so CAS can
//! only add catches, never block the bot.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use serde::Deserialize;
use teloxide::types::UserId;

use telegram_antispam_shield::config::CasConfig;

/// Cached results kept before expired ones are dropped.
const MAX_CACHED: usize = 10_000;

/// The part of the `/check` response we need: `ok` is `true` when listed.
#[derive(Deserialize)]
struct CheckResponse {
    ok: bool,
}

pub struct Cas {
    config: CasConfig,
    client: reqwest::Client,
    cache: Mutex<HashMap<u64, (bool, Instant)>>,
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
        let url = format!("{}/check", self.config.url.trim_end_matches('/'));
        let response = self
            .client
            .get(url)
            .query(&[("user_id", user.0)])
            .send()
            .await
            .and_then(|r| r.error_for_status());
        let listed = match response {
            Ok(r) => match r.json::<CheckResponse>().await {
                Ok(body) => body.ok,
                Err(e) => {
                    tracing::debug!(error = %e, "cas_bad_response");
                    return false;
                }
            },
            Err(e) => {
                tracing::debug!(error = %e, "cas_unreachable");
                return false;
            }
        };
        self.remember(user, listed);
        listed
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
