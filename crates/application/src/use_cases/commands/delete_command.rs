use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::use_cases::commands::get_command;
use crate::write_set::{Mutation, WriteSet};

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    application_id: Snowflake,
    guild_id: Option<Snowflake>,
    command_id: Snowflake,
) -> AppResult<()> {
    get_command::execute(ports, actor, application_id, guild_id, command_id).await?;
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::DeleteCommand { application_id, command_id });
    ports.unit_of_work.commit(writes).await
}
