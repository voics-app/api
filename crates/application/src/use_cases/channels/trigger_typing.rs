use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::events::DomainEvent;
use crate::support::channel_access::ChannelAccess;
use crate::support::views::member_profile;

/// Publishes TYPING_START directly (no persistence needed).
pub async fn execute(ports: &Ports, actor: &Actor, channel_id: Snowflake) -> AppResult<()> {
    let access = ChannelAccess::visible(ports, channel_id, actor.user_id).await?;
    access.require(Permissions::SEND_MESSAGES)?;
    let member = match &access.guild {
        Some(ctx) => Some(Box::new(member_profile(ports, ctx.member.clone()).await?)),
        None => None,
    };
    ports
        .events
        .publish(DomainEvent::TypingStart {
            channel_id,
            guild_id: access.channel.guild_id,
            user_id: actor.user_id,
            timestamp: ports.clock.now(),
            member,
        })
        .await
}
