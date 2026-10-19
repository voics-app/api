use chrono::{DateTime, Utc};
use domain::device::Device;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::use_cases::devices::devices_update;
use crate::write_set::{Mutation, WriteSet};

/// Removes one of the caller's devices; other users' devices are not found.
pub async fn execute(ports: &Ports, actor: &Actor, device_id: Snowflake) -> AppResult<()> {
    actor.require_user()?;
    let device = ports
        .devices
        .find(device_id)
        .await?
        .filter(|device| device.user_id == actor.user_id)
        .ok_or(AppError::NotFound(Resource::Device))?;
    remove(ports, device.user_id, device.id).await
}

/// Deletes a device and publishes the user's remaining device list.
pub async fn remove(ports: &Ports, user_id: Snowflake, device_id: Snowflake) -> AppResult<()> {
    let remaining: Vec<Device> = ports
        .devices
        .list_for_user(user_id)
        .await?
        .into_iter()
        .filter(|other| other.id != device_id)
        .collect();
    let event = devices_update::build(ports, user_id, remaining).await?;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::DeleteDevice { user_id, device_id })
        .emit(event);
    ports.unit_of_work.commit(writes).await
}

/// Called after one of a user's sessions is revoked: removes the device that
/// session was bound to, unless another live session still holds it.
pub async fn remove_if_unheld(
    ports: &Ports,
    user_id: Snowflake,
    device_id: Option<Snowflake>,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let Some(device_id) = device_id else {
        return Ok(());
    };
    if ports
        .sessions
        .live_device_ids(user_id, now)
        .await?
        .contains(&device_id)
    {
        return Ok(());
    }
    remove(ports, user_id, device_id).await
}
