use domain::channel::Channel;
use domain::guild::{GuildSnapshot, Member, MemberProfile};
use domain::user::User;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::{AppError, AppResult, Resource};
use crate::events::PrivateChannelView;
use crate::support::delivery::effective_mode;

/// Most members embedded in a `GuildFull` payload.
pub const SNAPSHOT_MEMBER_LIMIT: u32 = 1000;

pub async fn require_user(ports: &Ports, user_id: Snowflake) -> AppResult<User> {
    ports
        .users
        .find(user_id)
        .await?
        .ok_or(AppError::NotFound(Resource::User))
}

pub async fn private_view(ports: &Ports, channel: Channel) -> AppResult<PrivateChannelView> {
    let recipients = ports.users.find_many(&channel.recipient_ids).await?;
    let delivery_mode = effective_mode(ports, &channel).await?;
    Ok(PrivateChannelView {
        channel,
        recipients,
        delivery_mode,
    })
}

pub async fn member_profile(ports: &Ports, member: Member) -> AppResult<MemberProfile> {
    let user = require_user(ports, member.user_id).await?;
    Ok(MemberProfile { member, user })
}

pub async fn guild_snapshot(ports: &Ports, guild_id: Snowflake) -> AppResult<GuildSnapshot> {
    let guild = ports
        .guilds
        .find(guild_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Guild))?;
    let roles = ports.guilds.roles(guild_id).await?;
    let channels = ports.channels.list_for_guild(guild_id).await?;
    let members = ports
        .guilds
        .member_profiles(guild_id, SNAPSHOT_MEMBER_LIMIT, None)
        .await?;
    let emojis = ports.emojis.list(guild_id).await?;
    let member_count = ports.guilds.member_count(guild_id).await?;
    Ok(GuildSnapshot {
        guild,
        roles,
        channels,
        members,
        emojis,
        member_count,
    })
}
