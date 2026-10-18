//! The USER_DEVICES_UPDATE event shared by device registration and removal.

use std::collections::HashSet;

use domain::device::Device;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;
use crate::events::DomainEvent;

/// Carries the user's full device list. Targets the user plus every user who
/// shares a DM, group DM or guild with them, each listed once.
pub async fn build(
    ports: &Ports,
    user_id: Snowflake,
    devices: Vec<Device>,
) -> AppResult<DomainEvent> {
    let mut seen = HashSet::from([user_id]);
    let mut target_user_ids = vec![user_id];
    for id in ports.users.co_member_ids(user_id).await? {
        if seen.insert(id) {
            target_user_ids.push(id);
        }
    }
    Ok(DomainEvent::UserDevicesUpdate {
        user_id,
        devices,
        target_user_ids,
    })
}
