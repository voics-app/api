use crate::context::Ports;
use crate::error::{AppError, AppResult, Denial};
use crate::ports::RotateOutcome;
use crate::support::views::require_user;
use crate::use_cases::auth::tokens::{access_token, AuthTokens, REFRESH_TOKEN_BYTES};
use crate::use_cases::devices::delete_device;

/// Rotates the refresh token. Presenting an already-rotated token revokes
/// the whole session family (reuse detection).
pub async fn execute(ports: &Ports, refresh_token: &str) -> AppResult<AuthTokens> {
    let now = ports.clock.now();
    let old_hash = ports.secrets.hash(refresh_token);
    let new_token = ports.secrets.token(REFRESH_TOKEN_BYTES);
    let new_hash = ports.secrets.hash(&new_token);
    let session = match ports
        .sessions
        .rotate(
            &old_hash,
            &new_hash,
            now + ports.settings.refresh_token_ttl,
            now,
        )
        .await?
    {
        RotateOutcome::Rotated(session) => session,
        RotateOutcome::ReuseDetected(_) | RotateOutcome::Invalid => {
            return Err(AppError::Unauthorized)
        }
    };
    let user = require_user(ports, session.user_id).await?;
    if user.is_suspended(now) {
        ports.sessions.revoke(session.id, now).await?;
        ports.push_tokens.delete_for_session(session.id).await?;
        delete_device::remove_if_unheld(ports, user.id, session.device_id, now).await?;
        return Err(AppError::Forbidden(Denial::Suspended));
    }
    if let Some(device_id) = session.device_id {
        ports.devices.touch(device_id, now).await?;
    }
    let access_token = access_token(ports, user.id, session.id, session.device_id)?;
    Ok(AuthTokens {
        access_token,
        refresh_token: new_token,
        user,
    })
}
