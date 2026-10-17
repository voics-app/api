use domain::audit::AuditAction;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::guild_context::GuildContext;
use crate::support::views::require_user;
use crate::write_set::{Mutation, WriteSet};

/// Kick.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    user_id: Snowflake,
    audit: AuditContext<'_>,
) -> AppResult<()> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::KICK_MEMBERS)?;
    let target = ports.guilds.member(guild_id, user_id).await?.ok_or(AppError::NotFound(Resource::Member))?;
    ctx.require_outranks(&target)?;
    let user = require_user(ports, user_id).await?;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::DeleteMember { guild_id, user_id })
        .emit(DomainEvent::MemberRemove { guild_id, user })
        .emit(DomainEvent::GuildDelete { guild_id, recipients: Some(vec![user_id]) })
        .audit(audit.entry(ports, guild_id, AuditAction::MemberKick, Some(user_id), vec![])?);
    ports.unit_of_work.commit(writes).await
}
