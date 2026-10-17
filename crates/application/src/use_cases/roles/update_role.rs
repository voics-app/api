use serde_json::json;

use domain::audit::{AuditAction, AuditChange};
use domain::guild::Role;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::guild_context::GuildContext;
use crate::use_cases::roles::create_role::{apply_role_input, RoleInput};
use crate::write_set::{Mutation, WriteSet};

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    role_id: Snowflake,
    input: RoleInput,
    audit: AuditContext<'_>,
) -> AppResult<Role> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_ROLES)?;
    let before = ctx.role(role_id)?.clone();
    ctx.require_manageable_role(&before)?;
    if before.managed && (input.name.is_some() || input.permissions.is_some()) {
        return Err(AppError::BadRequest { code: 50028, message: "Managed roles cannot be renamed or re-permissioned.".into() });
    }
    if before.is_everyone() && input.name.is_some() {
        return Err(AppError::invalid("name", "ROLE_EVERYONE_NAME", "The @everyone role cannot be renamed."));
    }
    let mut role = before.clone();
    apply_role_input(&ctx, &mut role, input)?;
    let changes: Vec<AuditChange> = [
        AuditChange::if_changed("name", json!(before.name), json!(role.name)),
        AuditChange::if_changed(
            "permissions",
            json!(before.permissions.bits().to_string()),
            json!(role.permissions.bits().to_string()),
        ),
        AuditChange::if_changed("color", json!(before.color), json!(role.color)),
        AuditChange::if_changed("hoist", json!(before.hoist), json!(role.hoist)),
        AuditChange::if_changed("mentionable", json!(before.mentionable), json!(role.mentionable)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::UpdateRole(role.clone())).emit(DomainEvent::RoleUpdate(role.clone()));
    if !changes.is_empty() {
        writes.audit(audit.entry(ports, guild_id, AuditAction::RoleUpdate, Some(role.id), changes)?);
    }
    ports.unit_of_work.commit(writes).await?;
    Ok(role)
}
