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
    pub avatar: Option<Option<crate::support::images::ImageUpload>>,
    pub banner: Option<Option<crate::support::images::ImageUpload>>,
    pub bio: Option<Option<String>>,
    pub nick: Option<Option<String>>,
    pub roles: Option<Vec<Snowflake>>,
    pub mute: Option<bool>,
    pub deaf: Option<bool>,
    pub communication_disabled_until: Option<Option<DateTime<Utc>>>,
    /// `Some(Some(channel))` moves the member to a voice channel, `Some(None)` disconnects them.
    pub voice_channel: Option<Option<Snowflake>>,
    /// `false` promotes a current stage audience member; `true` moves them back to the audience.
    pub voice_suppress: Option<bool>,
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
    let before = ports
        .guilds
        .member(guild_id, user_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Member))?;
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
        for changed in roles
            .iter()
            .filter(|r| !before.role_ids.contains(r))
            .chain(before.role_ids.iter().filter(|r| !roles.contains(r)))
        {
            let role = ctx.role(*changed)?;
            if role.managed {
                return Err(AppError::BadRequest {
                    code: 50028,
                    message: "Managed roles cannot be assigned.".into(),
                });
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
    let voice = match input.voice_channel {
        Some(target) => Some(voice_move(ports, &ctx, guild_id, target).await?),
        None => None,
    };
    if input.voice_suppress.is_some() {
        ctx.require(Permissions::MUTE_MEMBERS)?;
    }
    let profile = super::profile::apply(
        ports,
        &mut member,
        super::profile::ProfileInput {
            actor_id: actor.user_id,
            avatar: input.avatar,
            banner: input.banner,
            bio: input.bio,
        },
    )
    .await?;
    commit_member_update(
        ports,
        &before,
        member,
        profile,
        voice,
        input.voice_suppress,
        audit,
    )
    .await
}

/// Checks that the caller may move members to `target` (or disconnect them for `None`).
///
/// The api does not track who is in voice: the gateway applies the move to the member's live
/// voice session and ignores it when they are not connected.
async fn voice_move(
    ports: &Ports,
    ctx: &GuildContext,
    guild_id: Snowflake,
    target: Option<Snowflake>,
) -> AppResult<Option<Snowflake>> {
    let Some(channel_id) = target else {
        ctx.require(Permissions::MOVE_MEMBERS)?;
        return Ok(None);
    };
    let channel = ports
        .channels
        .find(channel_id)
        .await?
        .filter(|c| c.guild_id == Some(guild_id))
        .ok_or(AppError::NotFound(Resource::Channel))?;
    if !channel.kind.is_voice() {
        return Err(AppError::invalid(
            "channel_id",
            "BASE_TYPE_CHOICES",
            "The channel must be a voice channel.",
        ));
    }
    let parent = match channel.parent_id {
        Some(parent_id) => ports.channels.find(parent_id).await?,
        None => None,
    };
    ctx.require_in_channel(
        &channel,
        parent.as_ref(),
        Permissions::MOVE_MEMBERS | Permissions::CONNECT,
    )?;
    Ok(Some(channel_id))
}
