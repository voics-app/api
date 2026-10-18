use domain::emoji::Emoji;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::support::guild_context::GuildContext;

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    emoji_id: Snowflake,
) -> AppResult<Emoji> {
    GuildContext::load(ports, guild_id, actor.user_id).await?;
    ports
        .emojis
        .find(guild_id, emoji_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Emoji))
}
