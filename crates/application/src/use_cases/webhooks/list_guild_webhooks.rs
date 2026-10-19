use domain::application::Webhook;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::support::guild_context::GuildContext;

pub async fn execute(ports: &Ports, actor: &Actor, guild_id: Snowflake) -> AppResult<Vec<Webhook>> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_WEBHOOKS)?;
    ports.webhooks.list_for_guild(guild_id).await
}
