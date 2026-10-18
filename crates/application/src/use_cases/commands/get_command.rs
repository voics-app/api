use domain::application::ApplicationCommand;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::use_cases::commands::command_rules::require_scope;

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    application_id: Snowflake,
    guild_id: Option<Snowflake>,
    command_id: Snowflake,
) -> AppResult<ApplicationCommand> {
    require_scope(ports, actor, application_id, guild_id).await?;
    ports
        .applications
        .command(application_id, command_id)
        .await?
        .filter(|c| c.guild_id == guild_id)
        .ok_or(AppError::NotFound(Resource::Command))
}
