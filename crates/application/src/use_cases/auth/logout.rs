use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::support::sessions::announce_revoked;
use crate::use_cases::devices::delete_device;

/// Revokes the caller's session and removes the device it was bound to.
pub async fn execute(ports: &Ports, actor: &Actor) -> AppResult<()> {
    actor.require_user()?;
    let Some(session_id) = actor.session_id() else {
        return Ok(());
    };
    let now = ports.clock.now();
    let device_id = ports
        .sessions
        .find(session_id)
        .await?
        .and_then(|session| session.device_id);
    ports.sessions.revoke(session_id, now).await?;
    delete_device::remove_if_unheld(ports, actor.user_id, device_id, now).await?;
    announce_revoked(ports, actor.user_id, vec![session_id]).await
}
