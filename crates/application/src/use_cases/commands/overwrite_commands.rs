use domain::application::{ApplicationCommand, MAX_COMMANDS_PER_SCOPE};
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::use_cases::commands::command_rules::{build, require_scope, CommandInput};
use crate::write_set::{Mutation, WriteSet};

/// `PUT .../commands`: replaces every command of the scope, keeping ids of
/// commands whose name and type are unchanged.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    application_id: Snowflake,
    guild_id: Option<Snowflake>,
    inputs: Vec<CommandInput>,
) -> AppResult<Vec<ApplicationCommand>> {
    require_scope(ports, actor, application_id, guild_id).await?;
    if inputs.len() > MAX_COMMANDS_PER_SCOPE * 3 {
        return Err(AppError::invalid("commands", "BASE_TYPE_MAX_LENGTH", "Too many commands."));
    }
    let existing = ports.applications.commands(application_id, guild_id).await?;
    let mut commands: Vec<ApplicationCommand> = Vec::with_capacity(inputs.len());
    for input in inputs {
        let draft = build(ports, application_id, guild_id, None, input.clone())?;
        if commands.iter().any(|c| c.name == draft.name && c.kind == draft.kind) {
            return Err(AppError::invalid("name", "APPLICATION_COMMAND_DUPLICATE", "Duplicate command name."));
        }
        let command = match existing.iter().find(|c| c.name == draft.name && c.kind == draft.kind) {
            Some(previous) => build(ports, application_id, guild_id, Some(previous), input)?,
            None => draft,
        };
        commands.push(command);
    }
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::ReplaceCommands { application_id, guild_id, commands: commands.clone() });
    ports.unit_of_work.commit(writes).await?;
    Ok(commands)
}
