use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::guild_context::GuildContext;
use crate::support::views::require_user;
use crate::write_set::{Mutation, WriteSet};

/// `DELETE /users/@me/guilds/{id}`. Owners must transfer or delete instead.
pub async fn execute(ports: &Ports, actor: &Actor, guild_id: Snowflake) -> AppResult<()> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    if ctx.is_owner() {
        return Err(AppError::BadRequest { code: 50055, message: "Owners cannot leave their guild.".into() });
    }
    let user = require_user(ports, actor.user_id).await?;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::DeleteMember { guild_id, user_id: actor.user_id })
        .emit(DomainEvent::MemberRemove { guild_id, user })
        .emit(DomainEvent::GuildDelete { guild_id, recipients: Some(vec![actor.user_id]) });
    ports.unit_of_work.commit(writes).await
}
