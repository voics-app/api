use crate::context::Ports;
use crate::error::{AppError, AppResult, Denial};
use crate::ports::{RefreshReplay, RotateOutcome};
use crate::support::views::require_user;
use crate::use_cases::auth::tokens::{access_token, AuthTokens, REFRESH_TOKEN_BYTES};
use crate::use_cases::devices::delete_device;

/// Rotates the refresh token. Presenting an already-rotated token revokes
/// the whole session family (reuse detection).
pub async fn execute(ports: &Ports, refresh_token: &str) -> AppResult<AuthTokens> {
    execute_with_recovery(ports, refresh_token, None).await
}

/// Recovery is deliberately limited to five minutes after a committed rotation.
/// Requests without an id keep the original strict single-use semantics.
pub async fn execute_with_recovery(
    ports: &Ports,
    refresh_token: &str,
    request_id: Option<uuid::Uuid>,
) -> AppResult<AuthTokens> {
    let now = ports.clock.now();
    let old_hash = ports.secrets.hash(refresh_token);
    let new_token = request_id.map_or_else(
        || ports.secrets.token(REFRESH_TOKEN_BYTES),
        |id| ports.secrets.retryable_refresh_token(refresh_token, id),
    );
    let new_hash = ports.secrets.hash(&new_token);
    let replay = request_id.map(|id| RefreshReplay {
        request_hash: ports.secrets.hash(&id.to_string()),
        earliest_rotation: now - chrono::Duration::minutes(5),
    });
    let session = match ports
        .sessions
        .rotate(
            &old_hash,
            &new_hash,
            now + ports.settings.refresh_token_ttl,
            now,
            replay.as_ref(),
        )
        .await?
    {
        RotateOutcome::Rotated(session) => session,
        RotateOutcome::ReuseDetected(session) => {
            delete_device::remove_if_unheld(ports, session.user_id, session.device_id, now).await?;
            return Err(AppError::Unauthorized);
        }
        RotateOutcome::Invalid => return Err(AppError::Unauthorized),
    };
    let user = require_user(ports, session.user_id).await?;
    if user.is_suspended(now) {
        ports.sessions.revoke(session.id, now).await?;
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
