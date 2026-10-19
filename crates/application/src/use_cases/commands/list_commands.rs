use domain::application::ApplicationCommand;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::use_cases::commands::command_rules::require_scope;

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    application_id: Snowflake,
    guild_id: Option<Snowflake>,
) -> AppResult<Vec<ApplicationCommand>> {
    require_scope(ports, actor, application_id, guild_id).await?;
    ports.applications.commands(application_id, guild_id).await
}
