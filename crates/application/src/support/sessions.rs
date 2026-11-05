//! Bulk session revocation with device cleanup and gateway disconnect.

use uuid::Uuid;

use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;
use crate::events::DomainEvent;
use crate::use_cases::devices::delete_device;
use crate::write_set::WriteSet;

/// Revokes the user's live sessions (all, or all but `keep`), removes the
/// devices that no surviving session holds and tells the gateway to drop the
/// revoked sessions' connections.
pub async fn revoke(ports: &Ports, user_id: Snowflake, keep: Option<Uuid>) -> AppResult<()> {
    let now = ports.clock.now();
    let held_before = ports.sessions.live_device_ids(user_id, now).await?;
    let revoked = match keep {
        Some(session_id) => {
            ports
                .sessions
                .revoke_others(user_id, session_id, now)
                .await?
        }
        None => ports.sessions.revoke_all_for_user(user_id, now).await?,
    };
    for device_id in held_before {
        delete_device::remove_if_unheld(ports, user_id, Some(device_id), now).await?;
    }
    announce_revoked(ports, user_id, revoked).await
}

/// Drops the push tokens of `session_ids` and publishes SESSION_REVOKE so the gateway
/// closes their connections. Nothing is published for an empty list.
pub async fn announce_revoked(
    ports: &Ports,
    user_id: Snowflake,
    session_ids: Vec<Uuid>,
) -> AppResult<()> {
    if session_ids.is_empty() {
        return Ok(());
    }
    ports.push_tokens.delete_for_sessions(&session_ids).await?;
    let mut writes = WriteSet::new();
    writes.emit(DomainEvent::SessionsRevoke {
        user_id,
        session_ids,
    });
    ports.unit_of_work.commit(writes).await
}
