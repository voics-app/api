use domain::guild::Role;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::support::guild_context::GuildContext;

pub async fn execute(ports: &Ports, actor: &Actor, guild_id: Snowflake) -> AppResult<Vec<Role>> {
    Ok(GuildContext::load(ports, guild_id, actor.user_id).await?.roles)
}
