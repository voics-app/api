use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};

/// The caller's own note about `target_id`; notes are never visible to anyone else.
pub async fn execute(ports: &Ports, actor: &Actor, target_id: Snowflake) -> AppResult<String> {
    actor.require_user()?;
    ports
        .users
        .note(actor.user_id, target_id)
        .await?
        .ok_or(AppError::NotFound(Resource::User))
}
