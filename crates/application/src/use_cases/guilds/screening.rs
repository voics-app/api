use domain::guild::MemberProfile;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::events::DomainEvent;
use crate::ports::ScreeningConfig;
use crate::support::guild_context::GuildContext;
use crate::support::views::require_user;
use crate::write_set::{Mutation, WriteSet};
use crate::{AppError, AppResult};

pub async fn get(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
) -> AppResult<(ScreeningConfig, bool)> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    Ok((ports.guilds.screening(guild_id).await?, ctx.member.pending))
}

pub async fn set(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    enabled: bool,
    rules: Vec<String>,
) -> AppResult<ScreeningConfig> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_GUILD)?;
    if rules.len() > 20
        || rules
            .iter()
            .any(|r| r.trim().is_empty() || r.chars().count() > 300)
        || (enabled && rules.is_empty())
    {
        return Err(AppError::invalid(
            "rules",
            "BASE_TYPE_BAD_LENGTH",
            "Provide 1–20 non-empty rules of at most 300 characters when enabled.",
        ));
    }
    let rules: Vec<String> = rules.into_iter().map(|r| r.trim().to_owned()).collect();
    ports.guilds.set_screening(guild_id, enabled, &rules).await
}

pub async fn accept(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    revision: i64,
) -> AppResult<()> {
    actor.require_user()?;
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    if !ctx.member.pending {
        return Ok(());
    }
    let user = require_user(ports, actor.user_id).await?;
    let mut member = ctx.member;
    member.pending = false;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::CheckScreeningRevision { guild_id, revision })
        .mutate(Mutation::AcceptScreening {
            guild_id,
            user_id: actor.user_id,
        })
        .emit(DomainEvent::MemberUpdate(MemberProfile { member, user }));
    ports.unit_of_work.commit(writes).await
}
