use domain::guild::MemberProfile;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::support::guild_context::GuildContext;
use crate::support::views::member_profile;

pub async fn execute(ports: &Ports, actor: &Actor, guild_id: Snowflake, user_id: Snowflake) -> AppResult<MemberProfile> {
    GuildContext::load(ports, guild_id, actor.user_id).await?;
    let member = ports.guilds.member(guild_id, user_id).await?.ok_or(AppError::NotFound(Resource::Member))?;
    member_profile(ports, member).await
}
