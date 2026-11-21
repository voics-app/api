use domain::channel::Channel;
use domain::guild::Guild;
use domain::invite::Invite;
use domain::user::User;

use crate::context::Ports;
use crate::error::{AppError, AppResult, Resource};

/// An invite with its guild, channel and inviter resolved.
#[derive(Debug, Clone)]
pub struct InviteView {
    pub invite: Invite,
    pub guild: Guild,
    pub channel: Channel,
    pub inviter: Option<User>,
    pub member_count: u64,
    /// Members online right now; `None` when presence is unavailable.
    pub presence_count: Option<u64>,
}

/// Members counted for the online figure; larger guilds report a lower bound.
const PRESENCE_SAMPLE: u32 = 10_000;

pub async fn view(ports: &Ports, invite: Invite) -> AppResult<InviteView> {
    let guild = ports
        .guilds
        .find(invite.guild_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Invite))?;
    let channel = ports
        .channels
        .find(invite.channel_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Invite))?;
    let inviter = match invite.inviter_id {
        Some(id) => ports.users.find(id).await?,
        None => None,
    };
    let member_count = ports.guilds.member_count(guild.id).await?;
    let presence_count = online_count(ports, guild.id).await;
    Ok(InviteView {
        invite,
        guild,
        channel,
        inviter,
        member_count,
        presence_count,
    })
}

/// Best effort: a presence outage must not break invite lookups.
pub(crate) async fn online_count(ports: &Ports, guild_id: domain::Snowflake) -> Option<u64> {
    let ids = ports
        .guilds
        .member_ids(guild_id, PRESENCE_SAMPLE)
        .await
        .ok()?;
    ports.presence.online_count(&ids).await.ok()
}

/// Public: anyone holding the code may look it up.
pub async fn execute(ports: &Ports, code: &str) -> AppResult<InviteView> {
    let (invite, _) = super::vanity::resolve(ports, code).await?;
    if !invite.is_usable(ports.clock.now()) {
        return Err(AppError::NotFound(Resource::Invite));
    }
    view(ports, invite).await
}
