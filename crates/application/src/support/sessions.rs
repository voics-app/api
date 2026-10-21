//! Bulk session revocation with device cleanup.

use uuid::Uuid;

use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;
use crate::use_cases::devices::delete_device;

/// Revokes the user's live sessions (all, or all but `keep`) and removes the
/// devices that no surviving session holds.
pub async fn revoke(ports: &Ports, user_id: Snowflake, keep: Option<Uuid>) -> AppResult<()> {
    let now = ports.clock.now();
    let held_before = ports.sessions.live_device_ids(user_id, now).await?;
    match keep {
        Some(session_id) => {
            ports
                .sessions
                .revoke_others(user_id, session_id, now)
                .await?
        }
        None => ports.sessions.revoke_all_for_user(user_id, now).await?,
    }
    for device_id in held_before {
        delete_device::remove_if_unheld(ports, user_id, Some(device_id), now).await?;
    }
    Ok(())
}
