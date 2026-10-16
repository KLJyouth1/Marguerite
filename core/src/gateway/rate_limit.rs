//! Production JSON-RPC token-bucket rate limiting for the gateway.
//! Global and per-host buckets are checked atomically.
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
pub const RATE_LIMITED: &str = "RATE_LIMITED";
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BucketConfig {
    capacity: u32,
    refill_per_second: f64,
}
impl BucketConfig {
    pub fn new(capacity: u32, refill_per_second: f64) -> Self {
        assert!(capacity > 0, "bucket capacity must be positive");
        assert!(
            refill_per_second >= 0.0 && refill_per_second.is_finite(),
            "refill rate must be finite and non-negative"
        );
        Self {
            capacity,
            refill_per_second,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitError {
    retry_after_ms: u64,
}
impl RateLimitError {
    fn new(retry_after_ms: u64) -> Self {
        Self { retry_after_ms }
    }
    pub fn code(&self) -> &'static str {
        RATE_LIMITED
    }
    pub fn retry_after_ms(&self) -> u64 {
        self.retry_after_ms
    }
    /// JSON-RPC error object. HTTP status codes are intentionally not involved.
    pub fn json_rpc_error(&self) -> Value {
        json!({
            "code": RATE_LIMITED,
            "message": "rate limit exceeded",
            "data": { "retry_after_ms": self.retry_after_ms, "protocol": "JSON-RPC" }
        })
    }
}
impl std::fmt::Display for RateLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{RATE_LIMITED}: retry after {} ms", self.retry_after_ms)
    }
}
impl std::error::Error for RateLimitError {}
#[derive(Debug, Clone)]
pub struct TokenBucket {
    config: BucketConfig,
    tokens: f64,
    last_refill: Instant,
}
impl TokenBucket {
    pub fn new(config: BucketConfig) -> Self {
        Self::at(Instant::now(), config.capacity, config.refill_per_second)
    }
    pub fn at(now: Instant, capacity: u32, refill_per_second: f64) -> Self {
        let config = BucketConfig::new(capacity, refill_per_second);
        Self {
            tokens: capacity as f64,
            config,
            last_refill: now,
        }
    }
    fn refill(&mut self, now: Instant) {
        let elapsed = now
            .checked_duration_since(self.last_refill)
            .unwrap_or(Duration::ZERO);
        if elapsed.is_zero() {
            return;
        }
        self.tokens = (self.tokens + elapsed.as_secs_f64() * self.config.refill_per_second)
            .min(self.config.capacity as f64);
        self.last_refill = now;
    }
    pub fn try_consume(&mut self, amount: u32) -> Result<(), RateLimitError> {
        self.try_consume_at(amount, Instant::now())
    }
    pub fn try_consume_at(&mut self, amount: u32, now: Instant) -> Result<(), RateLimitError> {
        if amount == 0 {
            return Ok(());
        }
        self.refill(now);
        let needed = amount as f64;
        if self.tokens + f64::EPSILON < needed {
            return Err(RateLimitError::new(retry_after_ms(
                needed - self.tokens,
                self.config.refill_per_second,
            )));
        }
        self.tokens -= needed;
        Ok(())
    }
    pub fn available(&mut self) -> u32 {
        self.refill(Instant::now());
        self.tokens.floor() as u32
    }
}
fn retry_after_ms(tokens_needed: f64, refill_per_second: f64) -> u64 {
    if refill_per_second <= 0.0 {
        return u64::MAX;
    }
    let ms = (tokens_needed / refill_per_second * 1000.0).ceil();
    ms.max(1.0).min(u64::MAX as f64) as u64
}
struct LimiterState {
    global: TokenBucket,
    hosts: HashMap<String, TokenBucket>,
}
pub struct RateLimiter {
    global_config: BucketConfig,
    host_config: BucketConfig,
    state: Mutex<LimiterState>,
}
impl RateLimiter {
    pub fn new(global: BucketConfig, per_host: BucketConfig) -> Self {
        Self::at(Instant::now(), global, per_host)
    }
    pub fn at(now: Instant, global: BucketConfig, per_host: BucketConfig) -> Self {
        Self {
            global_config: global,
            host_config: per_host,
            state: Mutex::new(LimiterState {
                global: TokenBucket::at(now, global.capacity, global.refill_per_second),
                hosts: HashMap::new(),
            }),
        }
    }
    pub fn check(&self, host: &str) -> Result<(), RateLimitError> {
        self.check_at(host, Instant::now())
    }
    pub fn check_at(&self, host: &str, now: Instant) -> Result<(), RateLimitError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Preflight both buckets first. No token is deducted until both pass.
        state.global.refill(now);
        let global_available = state.global.tokens >= 1.0 - f64::EPSILON;
        let host_bucket = state.hosts.entry(host.to_string()).or_insert_with(|| {
            TokenBucket::at(
                now,
                self.host_config.capacity,
                self.host_config.refill_per_second,
            )
        });
        host_bucket.refill(now);
        let host_tokens = host_bucket.tokens;
        let host_available = host_tokens >= 1.0 - f64::EPSILON;
        let global_tokens = state.global.tokens;
        if !global_available || !host_available {
            let global_retry = if global_available {
                0
            } else {
                retry_after_ms(1.0 - global_tokens, self.global_config.refill_per_second)
            };
            let host_retry = if host_available {
                0
            } else {
                retry_after_ms(1.0 - host_tokens, self.host_config.refill_per_second)
            };
            return Err(RateLimitError::new(global_retry.max(host_retry)));
        }
        state.global.tokens -= 1.0;
        // Reborrow only after the global bucket mutation to satisfy Rust's aliasing rules.
        state
            .hosts
            .get_mut(host)
            .expect("host bucket inserted above")
            .tokens -= 1.0;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
    fn token_bucket_capacity_and_over_limit() {
        let t0 = Instant::now();
        let mut bucket = TokenBucket::at(t0, 2, 1.0);
        assert!(bucket.try_consume_at(1, t0).is_ok());
        assert!(bucket.try_consume_at(1, t0).is_ok());
        let err = bucket.try_consume_at(1, t0).unwrap_err();
        assert_eq!(err.code(), RATE_LIMITED);
        assert!(err.retry_after_ms() >= 999);
    }
    #[test]
    fn retry_after_reports_wait_for_refill() {
        let t0 = Instant::now();
        let mut bucket = TokenBucket::at(t0, 1, 2.0);
        bucket.try_consume_at(1, t0).unwrap();
        let err = bucket
            .try_consume_at(1, t0 + Duration::from_millis(100))
            .unwrap_err();
        assert!(err.retry_after_ms() >= 399 && err.retry_after_ms() <= 500);
        let rpc = err.json_rpc_error();
        assert_eq!(rpc["code"], RATE_LIMITED);
        assert!(rpc["data"]["retry_after_ms"].as_u64().is_some());
        assert_eq!(rpc["data"]["protocol"], "JSON-RPC");
    }
    #[test]
    fn host_buckets_are_isolated() {
        let t0 = Instant::now();
        let limiter = RateLimiter::at(t0, BucketConfig::new(10, 1.0), BucketConfig::new(1, 0.0));
        assert!(limiter.check_at("host-a", t0).is_ok());
        assert!(limiter.check_at("host-b", t0).is_ok());
        assert_eq!(
            limiter.check_at("host-a", t0).unwrap_err().code(),
            RATE_LIMITED
        );
        assert_eq!(
            limiter.check_at("host-b", t0).unwrap_err().code(),
            RATE_LIMITED
        );
    }
    #[test]
    fn global_limit_applies_across_hosts() {
        let t0 = Instant::now();
        let limiter = RateLimiter::at(t0, BucketConfig::new(2, 0.0), BucketConfig::new(10, 0.0));
        limiter.check_at("host-a", t0).unwrap();
        limiter.check_at("host-b", t0).unwrap();
        let err = limiter.check_at("host-c", t0).unwrap_err();
        assert_eq!(err.code(), RATE_LIMITED);
    }
    #[test]
    fn rejection_does_not_consume_the_other_bucket() {
        let t0 = Instant::now();
        let limiter = RateLimiter::at(t0, BucketConfig::new(1, 1.0), BucketConfig::new(2, 0.0));
        limiter.check_at("host-a", t0).unwrap();
        let err = limiter.check_at("host-b", t0).unwrap_err();
        assert_eq!(err.code(), RATE_LIMITED);
        limiter
            .check_at("host-b", t0 + Duration::from_secs(1))
            .unwrap();
        limiter
            .check_at("host-b", t0 + Duration::from_secs(2))
            .unwrap();
        assert_eq!(
            limiter
                .check_at("host-b", t0 + Duration::from_secs(2))
                .unwrap_err()
                .code(),
            RATE_LIMITED
        );
    }
    #[test]
    fn refill_restores_capacity_using_monotonic_time() {
        let t0 = Instant::now();
        let mut bucket = TokenBucket::at(t0, 1, 1.0);
        bucket.try_consume_at(1, t0).unwrap();
        assert!(bucket
            .try_consume_at(1, t0 + Duration::from_millis(999))
            .is_err());
        assert!(bucket
            .try_consume_at(1, t0 + Duration::from_secs(1))
            .is_ok());
    }
    #[test]
    fn multiple_hosts_compete_for_global_tokens() {
        let t0 = Instant::now();
        let limiter = RateLimiter::at(t0, BucketConfig::new(1, 0.0), BucketConfig::new(1, 0.0));
        limiter.check_at("first", t0).unwrap();
        let err = limiter.check_at("second", t0).unwrap_err();
        assert_eq!(err.code(), RATE_LIMITED);
        assert!(err.retry_after_ms() > 0);
    }
}
