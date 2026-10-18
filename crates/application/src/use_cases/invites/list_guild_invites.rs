use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::support::guild_context::GuildContext;
use crate::use_cases::invites::get_invite::{view, InviteView};

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
) -> AppResult<Vec<InviteView>> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_GUILD)?;
    let now = ports.clock.now();
    let mut views = Vec::new();
    for invite in ports.invites.list_for_guild(guild_id).await? {
        if invite.is_usable(now) {
            views.push(view(ports, invite).await?);
        }
    }
    Ok(views)
}
