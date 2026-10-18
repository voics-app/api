use serde_json::json;

use domain::audit::{AuditAction, AuditChange};
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::guild_context::GuildContext;
use crate::write_set::{Mutation, WriteSet};

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    role_id: Snowflake,
    audit: AuditContext<'_>,
) -> AppResult<()> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_ROLES)?;
    let role = ctx.role(role_id)?.clone();
    if role.is_everyone() || role.managed {
        return Err(AppError::BadRequest {
            code: 50028,
            message: "This role cannot be deleted.".into(),
        });
    }
    ctx.require_manageable_role(&role)?;
    let changes = vec![AuditChange::new("name", Some(json!(role.name)), None)];
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::DeleteRole { guild_id, role_id })
        .emit(DomainEvent::RoleDelete { guild_id, role_id })
        .audit(audit.entry(
            ports,
            guild_id,
            AuditAction::RoleDelete,
            Some(role_id),
            changes,
        )?);
    ports.unit_of_work.commit(writes).await
}
