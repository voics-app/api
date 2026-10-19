use domain::audit::AuditAction;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::guild_context::GuildContext;
use crate::support::views::require_user;
use crate::write_set::{Mutation, WriteSet};

/// Bans a user (member or not) and removes their membership.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    user_id: Snowflake,
    audit: AuditContext<'_>,
) -> AppResult<()> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::BAN_MEMBERS)?;
    if user_id == actor.user_id || user_id == ctx.guild.owner_id {
        return Err(AppError::missing_permissions());
    }
    let user = require_user(ports, user_id).await?;
    let member = ports.guilds.member(guild_id, user_id).await?;
    if let Some(target) = &member {
        ctx.require_outranks(target)?;
    }
    if ports.guilds.ban(guild_id, user_id).await?.is_some() {
        return Ok(());
    }
    let mut writes = WriteSet::new();
    if member.is_some() {
        writes
            .mutate(Mutation::DeleteMember { guild_id, user_id })
            .emit(DomainEvent::MemberRemove {
                guild_id,
                user: user.clone(),
            })
            .emit(DomainEvent::GuildDelete {
                guild_id,
                recipients: Some(vec![user_id]),
            });
    }
    writes
        .mutate(Mutation::InsertBan {
            guild_id,
            user_id,
            reason: audit.reason.map(str::to_owned),
        })
        .emit(DomainEvent::BanAdd { guild_id, user })
        .audit(audit.entry(
            ports,
            guild_id,
            AuditAction::MemberBanAdd,
            Some(user_id),
            vec![],
        )?);
    ports.unit_of_work.commit(writes).await
}
