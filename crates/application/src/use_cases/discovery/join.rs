use domain::guild::{Member, MemberProfile};
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Denial, Resource};
use crate::events::DomainEvent;
use crate::support::views::{guild_snapshot, require_user};
use crate::use_cases::guilds::create_guild::MAX_GUILDS_PER_USER;
use crate::write_set::{Mutation, WriteSet};

/// `POST /discovery/guilds/{id}/join`: joins a guild that is publicly listed and
/// still qualifies. Joining a guild one is already in is a no-op.
pub async fn execute(ports: &Ports, actor: &Actor, guild_id: Snowflake) -> AppResult<()> {
    actor.require_user()?;
    let now = ports.clock.now();
    let guild = ports
        .guilds
        .find(guild_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Guild))?;
    let listed = ports.guilds.discovery(guild_id).await?.is_some();
    let qualifies = domain::discovery::ineligibilities(
        &guild,
        ports.guilds.member_count(guild_id).await?,
        ports.guilds.has_nsfw_channel(guild_id).await?,
        ports.settings.discovery_rules(),
        now,
    )
    .is_empty();
    if !listed || !qualifies {
        return Err(AppError::NotFound(Resource::Guild));
    }
    if ports
        .guilds
        .member(guild_id, actor.user_id)
        .await?
        .is_some()
    {
        return Ok(());
    }
    if ports.guilds.ban(guild_id, actor.user_id).await?.is_some() {
        return Err(AppError::Forbidden(Denial::Banned));
    }
    if ports.guilds.list_for_user(actor.user_id).await?.len() >= MAX_GUILDS_PER_USER {
        return Err(AppError::BadRequest {
            code: 30001,
            message: "Maximum number of guilds reached (200)".into(),
        });
    }

    let user = require_user(ports, actor.user_id).await?;
    let member = Member::new(guild_id, actor.user_id, now);
    let profile = MemberProfile {
        member: member.clone(),
        user,
    };
    let mut snapshot = guild_snapshot(ports, guild_id).await?;
    snapshot.members.push(profile.clone());
    snapshot.member_count += 1;

    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpsertMember(member))
        .emit(DomainEvent::MemberAdd(profile))
        .emit(DomainEvent::GuildCreate {
            snapshot: Box::new(snapshot),
            recipients: vec![actor.user_id],
        });
    ports.unit_of_work.commit(writes).await
}
