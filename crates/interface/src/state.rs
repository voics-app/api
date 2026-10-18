//! Shared handler state: the application ports, the internal shared secret
//! and the rate limiter that guards the public API.

use std::sync::Arc;
use std::time::Duration;

use application::ports::RateLimiter;
use application::Ports;

/// Requests allowed per identity and route bucket within one window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RatePolicy {
    pub limit: u32,
    pub window: Duration,
}

impl Default for RatePolicy {
    fn default() -> Self {
        Self {
            limit: 50,
            window: Duration::from_secs(5),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub ports: Ports,
    /// `INTERNAL_SERVICE_TOKEN` (P2), compared against `X-Internal-Token`.
    pub internal_token: Arc<str>,
    pub rate_limiter: Arc<dyn RateLimiter>,
    pub rate_policy: RatePolicy,
}
