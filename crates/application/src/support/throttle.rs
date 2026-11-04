//! Per-key throttles used inside use cases: second-factor attempts, password
//! confirmations and outbound mail.

use std::time::Duration;

use crate::context::Ports;
use crate::error::{AppError, AppResult};

/// Second-factor and backup-code attempts allowed per user in [`MFA_WINDOW`].
pub const MFA_ATTEMPTS: u32 = 10;
pub const MFA_WINDOW: Duration = Duration::from_secs(10 * 60);
/// Password confirmations allowed per user in [`PASSWORD_WINDOW`].
pub const PASSWORD_ATTEMPTS: u32 = 10;
pub const PASSWORD_WINDOW: Duration = Duration::from_secs(10 * 60);
/// Emails sent per recipient in [`MAIL_WINDOW`].
pub const MAIL_PER_RECIPIENT: u32 = 5;
pub const MAIL_WINDOW: Duration = Duration::from_secs(60 * 60);

/// Spends one attempt on `key`, failing with [`AppError::RateLimited`] when
/// the bucket is empty. The limiter falls back to per-pod memory buckets when its store fails
/// (`FallbackRateLimiter`), so only a limiter without that fallback lets the call through.
pub async fn hit(ports: &Ports, key: &str, limit: u32, window: Duration) -> AppResult<()> {
    match ports.limiter.hit(key, limit, window).await {
        Ok(decision) if !decision.allowed => Err(AppError::RateLimited {
            retry_after_secs: decision.retry_after.as_secs().max(1),
        }),
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::warn!(%error, key, "throttle store unavailable; attempt allowed");
            Ok(())
        }
    }
}
