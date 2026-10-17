use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};

/// Atomically takes one key package; each package is handed out once.
pub async fn execute(ports: &Ports, actor: &Actor, user_id: Snowflake, device_id: Snowflake) -> AppResult<Vec<u8>> {
    actor.require_user()?;
    let device = ports.devices.find(device_id).await?.ok_or(AppError::NotFound(Resource::Device))?;
    if device.user_id != user_id {
        return Err(AppError::NotFound(Resource::Device));
    }
    ports
        .devices
        .claim_key_package(device_id)
        .await?
        .ok_or(AppError::NotFound(Resource::KeyPackage))
}
