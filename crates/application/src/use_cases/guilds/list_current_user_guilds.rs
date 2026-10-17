use domain::guild::Guild;
use domain::permissions::Permissions;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::support::guild_context::GuildContext;

/// Partial guilds with the caller's guild-level permissions.
pub async fn execute(ports: &Ports, actor: &Actor) -> AppResult<Vec<(Guild, Permissions)>> {
    let guilds = ports.guilds.list_for_user(actor.user_id).await?;
    let mut out = Vec::with_capacity(guilds.len());
    for guild in guilds {
        let ctx = GuildContext::load(ports, guild.id, actor.user_id).await?;
        let permissions = ctx.permissions();
        out.push((ctx.guild, permissions));
    }
    Ok(out)
}
