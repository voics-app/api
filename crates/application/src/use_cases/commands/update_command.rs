use domain::application::ApplicationCommand;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::use_cases::commands::command_rules::{build, require_scope, CommandInput};
use crate::write_set::{Mutation, WriteSet};

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    application_id: Snowflake,
    guild_id: Option<Snowflake>,
    command_id: Snowflake,
    input: CommandInput,
) -> AppResult<ApplicationCommand> {
    require_scope(ports, actor, application_id, guild_id).await?;
    let existing = ports.applications.commands(application_id, guild_id).await?;
    let base = existing.iter().find(|c| c.id == command_id).ok_or(AppError::NotFound(Resource::Command))?;
    let command = build(ports, application_id, guild_id, Some(base), CommandInput { kind: None, ..input })?;
    if existing.iter().any(|c| c.id != command.id && c.name == command.name && c.kind == command.kind) {
        return Err(AppError::invalid("name", "APPLICATION_COMMAND_DUPLICATE", "A command with this name exists."));
    }
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::UpsertCommand(command.clone()));
    ports.unit_of_work.commit(writes).await?;
    Ok(command)
}
