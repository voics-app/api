use domain::guild::GuildSnapshot;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::support::guild_context::GuildContext;
use crate::support::views::guild_snapshot;

/// Guild with roles, emojis and counts (members are listed separately).
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
) -> AppResult<GuildSnapshot> {
    GuildContext::load(ports, guild_id, actor.user_id).await?;
    let mut snapshot = guild_snapshot(ports, guild_id).await?;
    snapshot.members.clear();
    snapshot.channels.clear();
    Ok(snapshot)
}
