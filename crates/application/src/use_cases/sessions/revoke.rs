use uuid::Uuid;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::use_cases::devices::delete_device;

/// `DELETE /users/@me/sessions/{id}`: revokes one of the caller's other live
/// sessions. Unknown, foreign and already revoked sessions are not found; the
/// current session is logged out through `POST /auth/logout` instead.
pub async fn execute(ports: &Ports, actor: &Actor, session_id: Uuid) -> AppResult<()> {
    actor.require_user()?;
    if actor.session_id() == Some(session_id) {
        return Err(AppError::invalid(
            "session_id",
            "CANNOT_REVOKE_CURRENT_SESSION",
            "Use logout to end the current session.",
        ));
    }
    let session = ports
        .sessions
        .find(session_id)
        .await?
        .filter(|session| session.user_id == actor.user_id && session.revoked_at.is_none())
        .ok_or(AppError::NotFound(Resource::Session))?;
    let now = ports.clock.now();
    ports.sessions.revoke(session.id, now).await?;
    delete_device::remove_if_unheld(ports, actor.user_id, session.device_id, now).await
}
