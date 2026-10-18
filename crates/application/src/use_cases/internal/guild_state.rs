use domain::guild::GuildSnapshot;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;
use crate::support::views::guild_snapshot;

/// `GET /internal/v1/guilds/{id}/state`.
pub async fn execute(ports: &Ports, guild_id: Snowflake) -> AppResult<GuildSnapshot> {
    guild_snapshot(ports, guild_id).await
}
