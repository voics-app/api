//! Periodic removal of devices nobody has seen for longer than the idle window (api#25).

use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;
use crate::use_cases::devices::devices_update;
use crate::write_set::{Mutation, WriteSet};

const BATCH: u32 = 200;

/// Deletes every device idle for longer than `Settings::device_max_idle` and publishes each
/// affected user's remaining device list. Returns how many devices were removed. Their key
/// packages go with them; sessions bound to them are unbound, as with eviction.
pub async fn execute(ports: &Ports) -> AppResult<usize> {
    let cutoff = ports.clock.now() - ports.settings.device_max_idle;
    let mut pruned = 0;
    loop {
        let stale = ports.devices.list_stale(cutoff, BATCH).await?;
        let mut users: Vec<Snowflake> = Vec::new();
        for device in &stale {
            if !users.contains(&device.user_id) {
                users.push(device.user_id);
            }
        }
        for user_id in users {
            let gone: Vec<Snowflake> = stale
                .iter()
                .filter(|device| device.user_id == user_id)
                .map(|device| device.id)
                .collect();
            let remaining = ports
                .devices
                .list_for_user(user_id)
                .await?
                .into_iter()
                .filter(|device| !gone.contains(&device.id))
                .collect();
            let mut writes = WriteSet::new();
            for device_id in &gone {
                writes.mutate(Mutation::DeleteDevice {
                    user_id,
                    device_id: *device_id,
                });
            }
            writes.emit(devices_update::build(ports, user_id, remaining).await?);
            ports.unit_of_work.commit(writes).await?;
            pruned += gone.len();
        }
        if stale.len() < BATCH as usize {
            return Ok(pruned);
        }
    }
}
