use domain::device::validate_key_packages;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    device_id: Snowflake,
    packages: Vec<Vec<u8>>,
) -> AppResult<()> {
    actor.require_user()?;
    let device = ports
        .devices
        .find(device_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Device))?;
    if device.user_id != actor.user_id {
        return Err(AppError::NotFound(Resource::Device));
    }
    validate_key_packages(&packages)?;
    ports.devices.add_key_packages(device_id, &packages).await
}
