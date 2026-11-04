use chrono::Duration;
use serde_json::json;

use domain::audit::{AuditAction, AuditChange};
use domain::permissions::Permissions;
use domain::{validation, Snowflake};

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::guild_context::GuildContext;
use crate::support::views::require_user;
use crate::write_set::{Mutation, WriteSet};

/// Who gets pruned: members inactive for `days` that hold no role except `include_roles`.
#[derive(Debug, Clone)]
pub struct PruneInput {
    pub days: u32,
    pub include_roles: Vec<Snowflake>,
}

/// KICK_MEMBERS, or ADMINISTRATOR once the guild restricts pruning to administrators.
fn authorize(ctx: &GuildContext) -> AppResult<()> {
    ctx.require(if ctx.guild.prune_admin_only {
        Permissions::ADMINISTRATOR
    } else {
        Permissions::KICK_MEMBERS
    })
}

async fn candidates(
    ports: &Ports,
    ctx: &GuildContext,
    input: &PruneInput,
) -> AppResult<Vec<Snowflake>> {
    validation::prune_days(input.days)?;
    let cutoff = ctx.now - Duration::days(i64::from(input.days));
    ports
        .guilds
        .prune_candidates(ctx.guild.id, cutoff, &input.include_roles)
        .await
}

/// `GET /guilds/{id}/prune`: how many members a prune would remove.
pub async fn count(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    input: PruneInput,
) -> AppResult<usize> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    authorize(&ctx)?;
    Ok(candidates(ports, &ctx, &input).await?.len())
}

/// `POST /guilds/{id}/prune`: removes them with one audit entry carrying the count.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    input: PruneInput,
    audit: AuditContext<'_>,
) -> AppResult<usize> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    authorize(&ctx)?;
    let removed = candidates(ports, &ctx, &input).await?;
    if removed.is_empty() {
        return Ok(0);
    }
    let mut writes = WriteSet::new();
    for user_id in &removed {
        let user = require_user(ports, *user_id).await?;
        writes
            .mutate(Mutation::DeleteMember {
                guild_id,
                user_id: *user_id,
            })
            .emit(DomainEvent::MemberRemove { guild_id, user })
            .emit(DomainEvent::GuildDelete {
                guild_id,
                recipients: Some(vec![*user_id]),
            });
    }
    writes.audit(audit.entry(
        ports,
        guild_id,
        AuditAction::MemberPrune,
        None,
        vec![
            AuditChange::new("delete_member_days", None, Some(json!(input.days))),
            AuditChange::new("members_removed", None, Some(json!(removed.len()))),
        ],
    )?);
    ports.unit_of_work.commit(writes).await?;
    Ok(removed.len())
}
