//! Liveness for container healthchecks.
//!
//! The poller writes the current Unix time to a heartbeat file after every
//! successful `getUpdates` call, that is at least every poll timeout (10 s).
//! `telegram-antispam-shield healthcheck` exits with 0 only while that time
//! is fresh, so the image needs neither a shell nor curl to be checked. A bot
//! that crashes, hangs or cannot reach Telegram turns unhealthy.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Where the heartbeat goes; the default is `antispam-heartbeat` in the
/// temp dir (`/tmp`, a tmpfs in compose.yaml since the root is read-only).
pub const PATH_ENV: &str = "ANTISPAM_HEARTBEAT_FILE";
/// Older heartbeats mean the poller has stopped: a long poll lasts 10 s and a
/// failed one is retried after 5 s, so this allows several misses in a row.
const MAX_AGE: Duration = Duration::from_secs(90);

fn path() -> &'static PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        std::env::var_os(PATH_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("antispam-heartbeat"))
    })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Record that polling works. A failure only costs the healthcheck.
pub fn beat() {
    if let Err(e) = std::fs::write(path(), unix_now().to_string()) {
        tracing::debug!(error = %e, path = %path().display(), "heartbeat_failed");
    }
}

/// Whether the last heartbeat is fresh; for the `healthcheck` subcommand.
pub fn check() -> bool {
    let beat = std::fs::read_to_string(path()).ok();
    is_fresh(beat.as_deref(), unix_now())
}

/// Whether heartbeat file contents `beat` are at most [`MAX_AGE`] old at `now`.
fn is_fresh(beat: Option<&str>, now: u64) -> bool {
    beat.and_then(|b| b.trim().parse::<u64>().ok())
        .is_some_and(|at| now.saturating_sub(at) <= MAX_AGE.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_recent_heartbeat_is_healthy() {
        let now = 1_761_335_100;
        assert!(is_fresh(Some("1761335100"), now));
        assert!(is_fresh(Some("1761335050\n"), now));
        assert!(!is_fresh(Some("1761330000"), now), "stale");
        assert!(!is_fresh(Some("garbage"), now));
        assert!(!is_fresh(None, now), "never beat");
    }
}
