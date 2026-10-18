use domain::guild::Ban;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::support::guild_context::GuildContext;

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    user_id: Snowflake,
) -> AppResult<Ban> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::BAN_MEMBERS)?;
    ports
        .guilds
        .ban(guild_id, user_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Ban))
}
