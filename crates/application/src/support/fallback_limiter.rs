//! Keeps rate limiting on when the shared limiter store fails (api#114). [`FallbackRateLimiter`] asks
//! the primary limiter first and, when it errors (Valkey down or at maxmemory), spends from a per-pod
//! in-memory token bucket instead of letting the request through. Every route goes through it, auth
//! routes included. The memory store holds at most [`MAX_FALLBACK_KEYS`] buckets, so an outage cannot
//! grow it without bound. Each pod enforces the limits on its own, so while the store is down the
//! effective limit is the configured one per pod.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::error::AppResult;
use crate::ports::{RateLimitDecision, RateLimiter};

/// Upper bound on the buckets the in-memory fallback keeps.
pub const MAX_FALLBACK_KEYS: usize = 50_000;

struct Bucket {
    tokens: f64,
    last: Instant,
    window: Duration,
}

impl Bucket {
    fn refilled_at(&self, limit: u32, now: Instant) -> f64 {
        let elapsed = now.saturating_duration_since(self.last);
        let rate = f64::from(limit) / self.window.as_secs_f64();
        (self.tokens + elapsed.as_secs_f64() * rate).min(f64::from(limit))
    }

    /// A bucket that has had a full window to refill is the same as no bucket.
    fn is_idle(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last) >= self.window
    }
}

/// Token buckets in process memory, bounded by `max_keys`.
pub struct MemoryRateLimiter {
    max_keys: usize,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl MemoryRateLimiter {
    pub fn new(max_keys: usize) -> Self {
        Self {
            max_keys,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    pub fn hit_at(
        &self,
        key: &str,
        limit: u32,
        window: Duration,
        now: Instant,
    ) -> RateLimitDecision {
        let window = window.max(Duration::from_millis(1));
        let capacity = f64::from(limit);
        let rate = capacity / window.as_secs_f64();
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        if !buckets.contains_key(key) && buckets.len() >= self.max_keys {
            make_room(&mut buckets, now);
        }
        let mut tokens = buckets
            .get(key)
            .map_or(capacity, |bucket| bucket.refilled_at(limit, now));
        let allowed = tokens >= 1.0;
        if allowed {
            tokens -= 1.0;
        }
        buckets.insert(
            key.to_owned(),
            Bucket {
                tokens,
                last: now,
                window,
            },
        );
        RateLimitDecision {
            allowed,
            limit,
            remaining: tokens.floor() as u32,
            reset_after: Duration::from_secs_f64((capacity - tokens) / rate),
            retry_after: if allowed {
                Duration::ZERO
            } else {
                Duration::from_secs_f64((1.0 - tokens) / rate)
            },
        }
    }
}

/// Drops idle buckets; when none are idle, drops the fullest one (the caller closest to unthrottled).
fn make_room(buckets: &mut HashMap<String, Bucket>, now: Instant) {
    buckets.retain(|_, bucket| !bucket.is_idle(now));
    let fullest = buckets
        .iter()
        .max_by(|a, b| a.1.tokens.total_cmp(&b.1.tokens))
        .map(|(key, _)| key.clone());
    if let Some(key) = fullest {
        buckets.remove(&key);
    }
}

#[async_trait]
impl RateLimiter for MemoryRateLimiter {
    async fn hit(&self, key: &str, limit: u32, window: Duration) -> AppResult<RateLimitDecision> {
        Ok(self.hit_at(key, limit, window, Instant::now()))
    }
}

/// `primary`, with an in-memory limiter taking over for every call the primary fails.
pub struct FallbackRateLimiter {
    primary: Arc<dyn RateLimiter>,
    fallback: MemoryRateLimiter,
}

impl FallbackRateLimiter {
    pub fn new(primary: Arc<dyn RateLimiter>) -> Self {
        Self {
            primary,
            fallback: MemoryRateLimiter::new(MAX_FALLBACK_KEYS),
        }
    }
}

#[async_trait]
impl RateLimiter for FallbackRateLimiter {
    async fn hit(&self, key: &str, limit: u32, window: Duration) -> AppResult<RateLimitDecision> {
        match self.primary.hit(key, limit, window).await {
            Ok(decision) => Ok(decision),
            Err(error) => {
                tracing::warn!(%error, "rate limiter store unavailable; using the in-memory fallback");
                let decision = self.fallback.hit_at(key, limit, window, Instant::now());
                let result = if decision.allowed {
                    "allowed"
                } else {
                    "limited"
                };
                metrics::counter!("rate_limit_fallback_total", "result" => result).increment(1);
                Ok(decision)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;

    const WINDOW: Duration = Duration::from_secs(10);

    fn allowed(limiter: &MemoryRateLimiter, key: &str, now: Instant) -> bool {
        limiter.hit_at(key, 3, WINDOW, now).allowed
    }

    #[test]
    fn spends_the_limit_then_throttles_with_a_retry_hint() {
        let limiter = MemoryRateLimiter::new(10);
        let now = Instant::now();
        for _ in 0..3 {
            assert!(allowed(&limiter, "a", now));
        }
        let denied = limiter.hit_at("a", 3, WINDOW, now);
        assert!(!denied.allowed);
        assert_eq!(denied.remaining, 0);
        assert!(denied.retry_after > Duration::ZERO && denied.retry_after <= WINDOW);
    }

    #[test]
    fn keys_do_not_share_tokens() {
        let limiter = MemoryRateLimiter::new(10);
        let now = Instant::now();
        for _ in 0..3 {
            allowed(&limiter, "a", now);
        }
        assert!(!allowed(&limiter, "a", now));
        assert!(allowed(&limiter, "b", now));
    }

    #[test]
    fn tokens_refill_over_the_window() {
        let limiter = MemoryRateLimiter::new(10);
        let now = Instant::now();
        for _ in 0..3 {
            allowed(&limiter, "a", now);
        }
        assert!(!allowed(&limiter, "a", now));
        let later = now + WINDOW / 3 + Duration::from_millis(50);
        assert!(allowed(&limiter, "a", later));
    }

    #[test]
    fn memory_stays_bounded_under_unique_keys() {
        let limiter = MemoryRateLimiter::new(4);
        let now = Instant::now();
        for i in 0..100 {
            allowed(&limiter, &format!("key-{i}"), now);
        }
        let held = limiter.buckets.lock().unwrap().len();
        assert!(held <= 4, "held {held} buckets");
    }

    #[test]
    fn a_full_store_evicts_the_fullest_bucket_not_the_drained_one() {
        let limiter = MemoryRateLimiter::new(2);
        let now = Instant::now();
        for _ in 0..3 {
            allowed(&limiter, "hot", now);
        }
        allowed(&limiter, "other", now);
        allowed(&limiter, "newcomer", now);
        assert!(!allowed(&limiter, "hot", now));
    }

    struct Failing;

    #[async_trait]
    impl RateLimiter for Failing {
        async fn hit(&self, _: &str, _: u32, _: Duration) -> AppResult<RateLimitDecision> {
            Err(AppError::Unavailable("valkey: OOM".into()))
        }
    }

    struct Fixed(RateLimitDecision);

    #[async_trait]
    impl RateLimiter for Fixed {
        async fn hit(&self, _: &str, _: u32, _: Duration) -> AppResult<RateLimitDecision> {
            Ok(self.0)
        }
    }

    #[tokio::test]
    async fn a_failing_primary_is_replaced_by_the_memory_limiter() {
        let limiter = FallbackRateLimiter::new(Arc::new(Failing));
        for _ in 0..3 {
            assert!(limiter.hit("login", 3, WINDOW).await.unwrap().allowed);
        }
        let denied = limiter.hit("login", 3, WINDOW).await.unwrap();
        assert!(!denied.allowed, "the outage must not remove the limit");
    }

    #[tokio::test]
    async fn a_working_primary_decides_alone() {
        let verdict = RateLimitDecision {
            allowed: false,
            limit: 3,
            remaining: 0,
            reset_after: WINDOW,
            retry_after: WINDOW,
        };
        let limiter = FallbackRateLimiter::new(Arc::new(Fixed(verdict)));
        assert_eq!(limiter.hit("k", 3, WINDOW).await.unwrap(), verdict);
    }

    #[tokio::test]
    async fn fallback_decisions_are_counted() {
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);
        let limiter = FallbackRateLimiter::new(Arc::new(Failing));
        for _ in 0..4 {
            limiter.hit("k", 3, WINDOW).await.unwrap();
        }
        let text = handle.render();
        assert!(
            text.contains(r#"rate_limit_fallback_total{result="allowed"} 3"#),
            "{text}"
        );
        assert!(
            text.contains(r#"rate_limit_fallback_total{result="limited"} 1"#),
            "{text}"
        );
    }
}
