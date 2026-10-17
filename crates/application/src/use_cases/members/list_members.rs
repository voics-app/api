use domain::guild::MemberProfile;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::support::guild_context::GuildContext;

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    limit: Option<u32>,
    after: Option<Snowflake>,
) -> AppResult<Vec<MemberProfile>> {
    GuildContext::load(ports, guild_id, actor.user_id).await?;
    let limit = limit.unwrap_or(1).clamp(1, 1000);
    ports.guilds.member_profiles(guild_id, limit, after).await
}
