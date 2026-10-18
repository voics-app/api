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
    let remaining: Vec<Device> = ports
        .devices
        .list_for_user(actor.user_id)
        .await?
        .into_iter()
        .filter(|other| other.id != device.id)
        .collect();
    let event = devices_update::build(ports, actor.user_id, remaining).await?;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::DeleteDevice {
            user_id: actor.user_id,
            device_id: device.id,
        })
        .emit(event);
    ports.unit_of_work.commit(writes).await
}
