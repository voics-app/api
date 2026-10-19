use domain::device::Device;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::support::views::require_user;

/// Public identity keys of a user's active devices (for MLS and safety numbers). Devices idle for
/// longer than the configured window are left out: nobody should encrypt to them.
pub async fn execute(ports: &Ports, actor: &Actor, user_id: Snowflake) -> AppResult<Vec<Device>> {
    actor.require_user()?;
    require_user(ports, user_id).await?;
    let now = ports.clock.now();
    let max_idle = ports.settings.device_max_idle;
    let mut devices = ports.devices.list_for_user(user_id).await?;
    devices.retain(|device| !device.is_stale(now, max_idle));
    Ok(devices)
}
