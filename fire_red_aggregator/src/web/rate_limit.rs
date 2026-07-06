//! In-memory per-IP rate limiting for the credential endpoints.
//!
//! `POST /api/login` and `POST /api/users` are the only unauthenticated
//! endpoints that run bcrypt, which makes each request cheap for an attacker
//! but expensive for the server. [`RateLimiter`] tracks hit timestamps per
//! client IP over a sliding window and rejects callers that exceed the limit,
//! before any bcrypt work is done.
//!
//! Note: keys come from axum's `ConnectInfo`, so behind a reverse proxy all
//! clients share the proxy's address (same caveat as the loopback guards in
//! `run_admin.rs`).

use fire_red_states::LockOrRecover;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Whole-map prune threshold: when the map holds more distinct IPs than this,
/// entries with no in-window hits are evicted to bound memory use.
const PRUNE_THRESHOLD: usize = 1024;

/// Sliding-window hit counter keyed by client IP.
///
/// Callers decide what a "hit" means: the login limiter records only
/// *failures* (so legitimate users are never throttled by their own successful
/// logins), while the registration limiter records every attempt.
pub(crate) struct RateLimiter {
    window: Duration,
    max_hits: usize,
    hits: Mutex<HashMap<IpAddr, Vec<Instant>>>,
}

impl RateLimiter {
    pub(crate) fn new(window: Duration, max_hits: usize) -> Self {
        Self {
            window,
            max_hits,
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// Returns `Err(retry_after_secs)` when `key` already has `max_hits`
    /// hits inside the window, `Ok(())` otherwise. Does not record a hit.
    pub(crate) fn check(&self, key: IpAddr) -> Result<(), u64> {
        self.check_at(key, Instant::now())
    }

    fn check_at(&self, key: IpAddr, now: Instant) -> Result<(), u64> {
        let mut map = self.hits.lock_or_recover();
        let Some(stamps) = map.get_mut(&key) else {
            return Ok(());
        };
        stamps.retain(|t| now.duration_since(*t) < self.window);
        if stamps.is_empty() {
            map.remove(&key);
            return Ok(());
        }
        if stamps.len() < self.max_hits {
            return Ok(());
        }
        // Full: the caller may retry once the oldest in-window hit expires.
        let oldest = stamps.iter().min().copied().unwrap_or(now);
        let retry_after = self
            .window
            .saturating_sub(now.duration_since(oldest))
            .as_secs()
            .max(1);
        Err(retry_after)
    }

    /// Records one hit for `key`.
    pub(crate) fn record(&self, key: IpAddr) {
        self.record_at(key, Instant::now());
    }

    fn record_at(&self, key: IpAddr, now: Instant) {
        let mut map = self.hits.lock_or_recover();
        if map.len() > PRUNE_THRESHOLD {
            let window = self.window;
            map.retain(|_, stamps| {
                stamps.retain(|t| now.duration_since(*t) < window);
                !stamps.is_empty()
            });
        }
        let stamps = map.entry(key).or_default();
        stamps.retain(|t| now.duration_since(*t) < self.window);
        // Cap per-key growth: once over the limit, extra hits carry no
        // additional information, so keep only the newest max_hits stamps.
        if stamps.len() >= self.max_hits {
            let excess = stamps.len() + 1 - self.max_hits;
            stamps.drain(..excess);
        }
        stamps.push(now);
    }

    /// Forgets all hits for `key`. Called after a successful login so a
    /// legitimate user who mistyped their password a few times starts fresh.
    pub(crate) fn clear(&self, key: IpAddr) {
        self.hits.lock_or_recover().remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, last))
    }

    #[test]
    fn allows_up_to_limit_then_rejects() {
        let rl = RateLimiter::new(Duration::from_secs(60), 3);
        let now = Instant::now();
        for _ in 0..3 {
            assert!(rl.check_at(ip(1), now).is_ok());
            rl.record_at(ip(1), now);
        }
        assert!(rl.check_at(ip(1), now).is_err());
    }

    #[test]
    fn keys_are_independent() {
        let rl = RateLimiter::new(Duration::from_secs(60), 1);
        let now = Instant::now();
        rl.record_at(ip(1), now);
        assert!(rl.check_at(ip(1), now).is_err());
        assert!(rl.check_at(ip(2), now).is_ok());
    }

    #[test]
    fn hits_expire_after_window() {
        let rl = RateLimiter::new(Duration::from_secs(60), 1);
        let start = Instant::now();
        rl.record_at(ip(1), start);
        assert!(rl.check_at(ip(1), start).is_err());
        assert!(rl.check_at(ip(1), start + Duration::from_secs(61)).is_ok());
    }

    #[test]
    fn retry_after_counts_down_as_time_passes() {
        let rl = RateLimiter::new(Duration::from_secs(60), 1);
        let start = Instant::now();
        rl.record_at(ip(1), start);
        let e1 = rl.check_at(ip(1), start).unwrap_err();
        let e2 = rl
            .check_at(ip(1), start + Duration::from_secs(50))
            .unwrap_err();
        assert!(e1 > e2, "retry-after should shrink: {e1} → {e2}");
        assert!(e2 >= 1, "retry-after is always at least 1s");
    }

    #[test]
    fn clear_resets_the_key() {
        let rl = RateLimiter::new(Duration::from_secs(60), 1);
        let now = Instant::now();
        rl.record_at(ip(1), now);
        assert!(rl.check_at(ip(1), now).is_err());
        rl.clear(ip(1));
        assert!(rl.check_at(ip(1), now).is_ok());
    }

    #[test]
    fn sliding_window_frees_slots_as_old_hits_age_out() {
        let rl = RateLimiter::new(Duration::from_secs(60), 2);
        let start = Instant::now();
        rl.record_at(ip(1), start);
        rl.record_at(ip(1), start + Duration::from_secs(30));
        assert!(rl.check_at(ip(1), start + Duration::from_secs(31)).is_err());
        // First hit ages out at +60s; one slot frees while the second remains.
        assert!(rl.check_at(ip(1), start + Duration::from_secs(61)).is_ok());
    }

    #[test]
    fn per_key_stamp_list_stays_bounded() {
        let rl = RateLimiter::new(Duration::from_secs(60), 3);
        let now = Instant::now();
        for _ in 0..100 {
            rl.record_at(ip(1), now);
        }
        assert_eq!(rl.hits.lock().unwrap()[&ip(1)].len(), 3);
    }

    #[test]
    fn whole_map_prunes_stale_keys_past_threshold() {
        let rl = RateLimiter::new(Duration::from_secs(60), 3);
        let start = Instant::now();
        for a in 0..=255u8 {
            for b in 0..5u8 {
                rl.record_at(IpAddr::V4(Ipv4Addr::new(10, 0, b, a)), start);
            }
        }
        assert!(rl.hits.lock().unwrap().len() > PRUNE_THRESHOLD);
        // A record long after the window should sweep the stale entries out.
        rl.record_at(ip(1), start + Duration::from_secs(120));
        assert!(rl.hits.lock().unwrap().len() <= 2);
    }
}
