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
}

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
    Ok(InviteView {
        invite,
        guild,
        channel,
        inviter,
        member_count,
    })
}

/// Public: anyone holding the code may look it up.
pub async fn execute(ports: &Ports, code: &str) -> AppResult<InviteView> {
    let invite = ports
        .invites
        .find(code)
        .await?
        .ok_or(AppError::NotFound(Resource::Invite))?;
    if !invite.is_usable(ports.clock.now()) {
        return Err(AppError::NotFound(Resource::Invite));
    }
    view(ports, invite).await
}
