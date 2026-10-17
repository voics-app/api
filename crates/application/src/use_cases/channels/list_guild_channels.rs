use domain::channel::Channel;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::support::guild_context::GuildContext;

/// Channels the caller can see (threads excluded, as in Discord).
pub async fn execute(ports: &Ports, actor: &Actor, guild_id: Snowflake) -> AppResult<Vec<Channel>> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    let channels = ports.channels.list_for_guild(guild_id).await?;
    Ok(channels
        .into_iter()
        .filter(|c| !c.kind.is_thread())
        .filter(|c| ctx.channel_permissions(c, None).contains(Permissions::VIEW_CHANNEL))
        .collect())
}
