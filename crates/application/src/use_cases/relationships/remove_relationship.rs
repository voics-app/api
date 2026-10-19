use domain::relationship;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::support::relationships::{apply_change, current_pair};
use crate::support::views::require_user;
use crate::write_set::WriteSet;

/// Unfriend, cancel or decline a request, or unblock.
pub async fn execute(ports: &Ports, actor: &Actor, target_id: Snowflake) -> AppResult<()> {
    actor.require_user()?;
    let me = require_user(ports, actor.user_id).await?;
    let target = require_user(ports, target_id).await?;
    let before = current_pair(ports, &me, &target).await?;
    let change = relationship::remove(before.0, before.1);
    let mut writes = WriteSet::new();
    apply_change(ports, &mut writes, &me, &target, before, change);
    if writes.is_empty() {
        return Ok(());
    }
    ports.unit_of_work.commit(writes).await
}
