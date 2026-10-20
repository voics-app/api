//! The USER_DEVICES_UPDATE event shared by device registration and removal.

use std::collections::HashSet;

use domain::device::Device;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;
use crate::events::DomainEvent;

/// Carries the user's full list of active devices (stale ones are left out, so peers stop
/// fanning out to them). Targets the user plus every user who
/// shares a DM, group DM or guild with them, each listed once.
pub async fn build(
    ports: &Ports,
    user_id: Snowflake,
    devices: Vec<Device>,
) -> AppResult<DomainEvent> {
    let now = ports.clock.now();
    let max_idle = ports.settings.device_max_idle;
    let devices: Vec<Device> = devices
        .into_iter()
        .filter(|device| !device.is_stale(now, max_idle))
        .collect();
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
