use domain::relationship::{self, RelationshipError, RelationshipType};
use domain::user::User;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::support::relationships::{apply_change, current_pair};
use crate::support::views::require_user;
use crate::write_set::WriteSet;

/// `PUT /users/@me/relationships/{id}`: `kind` 2 blocks, anything else
/// sends or accepts a friend request.
pub async fn execute(ports: &Ports, actor: &Actor, target_id: Snowflake, kind: Option<u8>) -> AppResult<()> {
    let target = require_user(ports, target_id).await?;
    to_user(ports, actor, &target, kind).await
}

pub(crate) async fn to_user(ports: &Ports, actor: &Actor, target: &User, kind: Option<u8>) -> AppResult<()> {
    actor.require_user()?;
    if target.id == actor.user_id {
        return Err(RelationshipError::SelfTarget.into());
    }
    if target.bot {
        return Err(RelationshipError::BotTarget.into());
    }
    let me = require_user(ports, actor.user_id).await?;
    let before = current_pair(ports, &me, target).await?;
    let change = match kind.map(RelationshipType::from_code) {
        Some(Some(RelationshipType::Blocked)) => relationship::block(before.1),
        None | Some(Some(RelationshipType::Friend)) => relationship::send_friend_request(before.0, before.1)?,
        Some(_) => return Err(AppError::invalid("type", "BASE_TYPE_CHOICES", "Must be 1 or 2.")),
    };
    let mut writes = WriteSet::new();
    apply_change(ports, &mut writes, &me, target, before, change);
    if writes.is_empty() {
        return Ok(());
    }
    ports.unit_of_work.commit(writes).await
}
