//! Token-bucket rate limiter (LLD-08 §6 step 3): 60 tokens, refill 1/sec,
//! keyed by `(client_pid, tool_name)`.
//!
//! One `mnemos-mcp-server` process serves exactly one stdio client for its
//! whole lifetime (LLD-08 §4 — "each [external client] gets its own
//! `mnemos-mcp-server` process"), so `client_pid` is a constant for this
//! process and the bucket only needs to be keyed by tool name.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

const CAPACITY: f64 = 60.0;
const REFILL_PER_SEC: f64 = 1.0;

struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

pub struct RateLimiter {
    buckets: Mutex<HashMap<&'static str, Bucket>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Returns `Ok(())` if the call may proceed, or `Err(retry_after_ms)`.
    pub fn try_acquire(&self, tool: &'static str) -> Result<(), u64> {
        let mut buckets = self.buckets.lock().expect("rate limiter mutex poisoned");
        let now = Instant::now();
        let bucket = buckets.entry(tool).or_insert_with(|| Bucket {
            tokens: CAPACITY,
            last_refill: now,
        });

        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * REFILL_PER_SEC).min(CAPACITY);
        bucket.last_refill = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            let deficit = 1.0 - bucket.tokens;
            let retry_after_ms = (deficit / REFILL_PER_SEC * 1000.0).ceil() as u64;
            Err(retry_after_ms)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixty_calls_in_a_burst_never_reject_and_the_sixty_first_does() {
        let limiter = RateLimiter::new();
        for _ in 0..60 {
            assert!(limiter.try_acquire("mnemos.search").is_ok());
        }
        let err = limiter.try_acquire("mnemos.search").unwrap_err();
        assert!(err > 0);
    }

    #[test]
    fn buckets_are_independent_per_tool() {
        let limiter = RateLimiter::new();
        for _ in 0..60 {
            limiter.try_acquire("mnemos.search").unwrap();
        }
        assert!(limiter.try_acquire("mnemos.list_projects").is_ok());
    }
}
