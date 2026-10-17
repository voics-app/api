use domain::guild::Role;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::guild_context::GuildContext;
use crate::write_set::{Mutation, WriteSet};

/// `PATCH /guilds/{id}/roles` with `[{id, position}]`. Roles at or above the
/// caller's highest role cannot be moved, nor moved to such a position.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    positions: Vec<(Snowflake, i32)>,
) -> AppResult<Vec<Role>> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_ROLES)?;
    let ceiling = if ctx.is_owner() { i32::MAX } else { ctx.highest_position() };
    let mut roles = ctx.roles.clone();
    let mut writes = WriteSet::new();
    for (role_id, position) in positions {
        let role = roles.iter_mut().find(|r| r.id == role_id).ok_or_else(|| {
            AppError::invalid("id", "ROLE_INVALID", "Unknown role.")
        })?;
        if role.is_everyone() || position < 1 || role.position >= ceiling || position >= ceiling {
            return Err(AppError::missing_permissions());
        }
        role.position = position;
        writes.mutate(Mutation::UpdateRole(role.clone())).emit(DomainEvent::RoleUpdate(role.clone()));
    }
    if !writes.is_empty() {
        ports.unit_of_work.commit(writes).await?;
    }
    roles.sort_by_key(|r| r.position);
    Ok(roles)
}
