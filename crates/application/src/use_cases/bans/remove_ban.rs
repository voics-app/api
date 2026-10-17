use domain::audit::AuditAction;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::guild_context::GuildContext;
use crate::write_set::{Mutation, WriteSet};

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    user_id: Snowflake,
    audit: AuditContext<'_>,
) -> AppResult<()> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::BAN_MEMBERS)?;
    let ban = ports.guilds.ban(guild_id, user_id).await?.ok_or(AppError::NotFound(Resource::Ban))?;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::DeleteBan { guild_id, user_id })
        .emit(DomainEvent::BanRemove { guild_id, user: ban.user })
        .audit(audit.entry(ports, guild_id, AuditAction::MemberBanRemove, Some(user_id), vec![])?);
    ports.unit_of_work.commit(writes).await
}
