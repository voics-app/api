use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::guild_context::GuildContext;
use crate::write_set::{Mutation, WriteSet};

/// Only the owner can delete a guild.
pub async fn execute(ports: &Ports, actor: &Actor, guild_id: Snowflake) -> AppResult<()> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    if !ctx.is_owner() {
        return Err(AppError::missing_permissions());
    }
    let mut writes = WriteSet::new();
    writes
        .emit(DomainEvent::GuildDelete { guild_id, recipients: None })
        .mutate(Mutation::DeleteGuild(guild_id));
    ports.unit_of_work.commit(writes).await
}
