use domain::application::ApplicationCommand;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::use_cases::commands::command_rules::{build, ensure_capacity, require_scope, CommandInput};
use crate::write_set::{Mutation, WriteSet};

/// Creating a command whose name and type already exist overwrites it.
/// Returns the command and whether it is new (201 vs 200).
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    application_id: Snowflake,
    guild_id: Option<Snowflake>,
    input: CommandInput,
) -> AppResult<(ApplicationCommand, bool)> {
    require_scope(ports, actor, application_id, guild_id).await?;
    let existing = ports.applications.commands(application_id, guild_id).await?;
    let draft = build(ports, application_id, guild_id, None, input.clone())?;
    let previous = existing.iter().find(|c| c.name == draft.name && c.kind == draft.kind);
    let command = match previous {
        Some(previous) => build(ports, application_id, guild_id, Some(previous), input)?,
        None => {
            ensure_capacity(&existing, draft.kind)?;
            draft
        }
    };
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::UpsertCommand(command.clone()));
    ports.unit_of_work.commit(writes).await?;
    Ok((command, previous.is_none()))
}
