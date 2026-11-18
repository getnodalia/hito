//! A small in-memory rate limiter: one token bucket per key, refilled continuously.
//!
//! It lives in the server process, which is right for a single-node deployment. Keys are
//! short strings such as `auth:<address>` or `write:<user id>`.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Buckets kept before stale ones are swept.
const SWEEP_AT: usize = 50_000;

#[derive(Debug, Default)]
pub struct Limiter {
    buckets: Mutex<HashMap<String, (f64, Instant)>>,
}

impl Limiter {
    /// Take one token from `key`'s bucket, which holds `per_minute` tokens and refills at
    /// `per_minute` per minute. Returns false when it is empty.
    pub fn allow(&self, key: &str, per_minute: u32) -> bool {
        if per_minute == 0 {
            return false;
        }
        let capacity = f64::from(per_minute);
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        if buckets.len() > SWEEP_AT {
            // Every bucket refills completely in one minute, regardless of its rate.
            // The current request's capacity must not be used to estimate other
            // buckets: those can have different limits and still be depleted.
            buckets.retain(|_, (_, at)| now.duration_since(*at) < Duration::from_secs(60));
        }
        let (tokens, at) = buckets.entry(key.to_owned()).or_insert((capacity, now));
        let refill = now.duration_since(*at).as_secs_f64() * capacity / 60.0;
        *tokens = (*tokens + refill).min(capacity);
        *at = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bucket_empties_and_keys_are_independent() {
        let l = Limiter::default();
        for _ in 0..3 {
            assert!(l.allow("a", 3));
        }
        assert!(!l.allow("a", 3));
        assert!(l.allow("b", 3));
        assert!(!l.allow("c", 0));
    }

    #[test]
    fn sweeping_a_low_rate_bucket_preserves_a_recent_partly_used_high_rate_bucket() {
        let l = Limiter::default();
        assert!(l.allow("read", 3));
        {
            let mut buckets = l.buckets.lock().unwrap();
            // Force the next call to sweep without making the read bucket stale.
            for i in 0..SWEEP_AT {
                buckets.insert(format!("filler:{i}"), (0.0, Instant::now()));
            }
        }
        assert!(l.allow("auth", 1));
        assert!(l.allow("read", 3));
        assert!(l.allow("read", 3));
        assert!(!l.allow("read", 3));
    }

    #[test]
    fn sweeping_removes_buckets_idle_for_a_full_refill() {
        let l = Limiter::default();
        {
            let mut buckets = l.buckets.lock().unwrap();
            let stale = Instant::now() - Duration::from_secs(61);
            for i in 0..=SWEEP_AT {
                buckets.insert(format!("stale:{i}"), (0.0, stale));
            }
        }
        assert!(l.allow("new", 1));
        assert_eq!(l.buckets.lock().unwrap().len(), 1);
    }
}
