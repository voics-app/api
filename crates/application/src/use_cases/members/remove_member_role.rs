use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::support::audit::AuditContext;
use crate::use_cases::members::update_member::{self, UpdateMemberInput};

/// `DELETE /guilds/{id}/members/{uid}/roles/{rid}`.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    user_id: Snowflake,
    role_id: Snowflake,
    audit: AuditContext<'_>,
) -> AppResult<()> {
    let member = ports.guilds.member(guild_id, user_id).await?.ok_or(AppError::NotFound(Resource::Member))?;
    if !member.role_ids.contains(&role_id) {
        return Ok(());
    }
    let roles = member.role_ids.iter().copied().filter(|r| *r != role_id).collect();
    let input = UpdateMemberInput { roles: Some(roles), ..Default::default() };
    update_member::execute(ports, actor, guild_id, user_id, input, audit).await.map(|_| ())
}
