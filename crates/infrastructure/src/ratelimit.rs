//! Token-bucket rate limiter in Valkey (P9). Each bucket is a hash holding
//! its token count and last refill time; the check-and-take runs as one Lua
//! script so concurrent api replicas cannot both spend the last token.

use std::time::Duration;

use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::Script;

use application::ports::{RateLimitDecision, RateLimiter};
use application::{AppError, AppResult};

/// KEYS[1] bucket; ARGV: capacity, window (ms), now (ms).
/// Returns { allowed, remaining, reset_after_ms, retry_after_ms }.
const TOKEN_BUCKET: &str = r#"
local capacity = tonumber(ARGV[1])
local window = tonumber(ARGV[2])
local now = tonumber(ARGV[3])
local rate = capacity / window
local state = redis.call('HMGET', KEYS[1], 'tokens', 'ts')
local tokens = tonumber(state[1])
local last = tonumber(state[2])
if tokens == nil then
  tokens = capacity
  last = now
end
tokens = math.min(capacity, tokens + math.max(0, now - last) * rate)
local allowed = 0
if tokens >= 1 then
  tokens = tokens - 1
  allowed = 1
end
redis.call('HSET', KEYS[1], 'tokens', tokens, 'ts', now)
redis.call('PEXPIRE', KEYS[1], window)
local reset_after = math.ceil((capacity - tokens) / rate)
local retry_after = 0
if allowed == 0 then
  retry_after = math.ceil((1 - tokens) / rate)
end
return { allowed, math.floor(tokens), reset_after, retry_after }
"#;

pub struct ValkeyRateLimiter {
    connection: ConnectionManager,
    script: Script,
}

impl ValkeyRateLimiter {
    pub async fn connect(url: &str) -> AppResult<Self> {
        let client = redis::Client::open(url).map_err(AppError::internal)?;
        let connection = ConnectionManager::new(client)
            .await
            .map_err(|e| AppError::Unavailable(format!("valkey: {e}")))?;
        Ok(Self {
            connection,
            script: Script::new(TOKEN_BUCKET),
        })
    }
}

#[async_trait]
impl RateLimiter for ValkeyRateLimiter {
    async fn hit(&self, key: &str, limit: u32, window: Duration) -> AppResult<RateLimitDecision> {
        let now_ms = u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or_default();
        let window_ms = u64::try_from(window.as_millis()).unwrap_or(u64::MAX).max(1);
        let mut connection = self.connection.clone();
        let (allowed, remaining, reset_ms, retry_ms): (i64, i64, i64, i64) = self
            .script
            .key(key)
            .arg(limit)
            .arg(window_ms)
            .arg(now_ms)
            .invoke_async(&mut connection)
            .await
            .map_err(|e| AppError::Unavailable(format!("valkey rate limit: {e}")))?;
        Ok(RateLimitDecision {
            allowed: allowed == 1,
            limit,
            remaining: u32::try_from(remaining.max(0)).unwrap_or(0),
            reset_after: Duration::from_millis(u64::try_from(reset_ms.max(0)).unwrap_or(0)),
            retry_after: Duration::from_millis(u64::try_from(retry_ms.max(0)).unwrap_or(0)),
        })
    }
}
