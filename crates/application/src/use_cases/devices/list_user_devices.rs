use domain::device::Device;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::support::views::require_user;

/// Public identity keys of a user's devices (for MLS and safety numbers).
pub async fn execute(ports: &Ports, actor: &Actor, user_id: Snowflake) -> AppResult<Vec<Device>> {
    actor.require_user()?;
    require_user(ports, user_id).await?;
    ports.devices.list_for_user(user_id).await
}
