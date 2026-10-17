use chrono::{DateTime, Duration, Utc};

use domain::guild::MemberProfile;
use domain::permissions::Permissions;
use domain::validation;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::support::audit::AuditContext;
use crate::support::guild_context::GuildContext;
use crate::use_cases::members::member_changes::commit_member_update;

/// Timeouts may last at most 28 days.
pub const MAX_TIMEOUT_DAYS: i64 = 28;

#[derive(Debug, Clone, Default)]
pub struct UpdateMemberInput {
    pub nick: Option<Option<String>>,
    pub roles: Option<Vec<Snowflake>>,
    pub mute: Option<bool>,
    pub deaf: Option<bool>,
    pub communication_disabled_until: Option<Option<DateTime<Utc>>>,
}

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    user_id: Snowflake,
    input: UpdateMemberInput,
    audit: AuditContext<'_>,
) -> AppResult<MemberProfile> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    let before = ports.guilds.member(guild_id, user_id).await?.ok_or(AppError::NotFound(Resource::Member))?;
    let is_self = user_id == actor.user_id;
    let mut member = before.clone();

    if let Some(nick) = input.nick {
        if is_self {
            ctx.require(Permissions::CHANGE_NICKNAME)?;
        } else {
            ctx.require(Permissions::MANAGE_NICKNAMES)?;
            ctx.require_outranks(&before)?;
        }
        if let Some(nick) = &nick {
            validation::nickname(nick)?;
        }
        member.nick = nick.map(|n| n.trim().to_owned());
    }
    if let Some(mut roles) = input.roles {
        ctx.require(Permissions::MANAGE_ROLES)?;
        roles.sort();
        roles.dedup();
        roles.retain(|r| *r != guild_id);
        for changed in roles.iter().filter(|r| !before.role_ids.contains(r)).chain(
            before.role_ids.iter().filter(|r| !roles.contains(r)),
        ) {
            let role = ctx.role(*changed)?;
            if role.managed {
                return Err(AppError::BadRequest { code: 50028, message: "Managed roles cannot be assigned.".into() });
            }
            ctx.require_manageable_role(role)?;
        }
        member.role_ids = roles;
    }
    if let Some(mute) = input.mute {
        ctx.require(Permissions::MUTE_MEMBERS)?;
        member.mute = mute;
    }
    if let Some(deaf) = input.deaf {
        ctx.require(Permissions::DEAFEN_MEMBERS)?;
        member.deaf = deaf;
    }
    if let Some(until) = input.communication_disabled_until {
        ctx.require(Permissions::MODERATE_MEMBERS)?;
        ctx.require_outranks(&before)?;
        if until.is_some_and(|u| u > ports.clock.now() + Duration::days(MAX_TIMEOUT_DAYS)) {
            return Err(AppError::invalid(
                "communication_disabled_until",
                "BASE_TYPE_BAD_DATE",
                "Timeouts can last at most 28 days.",
            ));
        }
        member.communication_disabled_until = until;
    }
    commit_member_update(ports, &before, member, audit).await
}
