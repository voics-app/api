use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::support::guild_context::GuildContext;
use domain::application::FollowedChannel;
use domain::permissions::Permissions;
use domain::Snowflake;

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
) -> AppResult<Vec<FollowedChannel>> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_WEBHOOKS)?;
    ports.webhooks.list_followed_for_guild(guild_id).await
}
