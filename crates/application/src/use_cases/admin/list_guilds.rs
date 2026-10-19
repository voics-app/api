use domain::guild::Guild;

use crate::context::Ports;
use crate::error::AppResult;

pub const GUILD_LIMIT: u32 = 100;

/// `GET /internal/v1/admin/guilds?query=`: guilds with their member counts.
pub async fn execute(ports: &Ports, query: Option<&str>) -> AppResult<Vec<(Guild, u64)>> {
    let query = query.map(str::trim).filter(|q| !q.is_empty());
    let guilds = ports.guilds.search(query, GUILD_LIMIT).await?;
    let mut out = Vec::with_capacity(guilds.len());
    for guild in guilds {
        let members = ports.guilds.member_count(guild.id).await?;
        out.push((guild, members));
    }
    Ok(out)
}
