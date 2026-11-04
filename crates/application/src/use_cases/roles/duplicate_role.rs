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
use crate::use_cases::roles::create_role::MAX_ROLES_PER_GUILD;
use crate::write_set::{Mutation, WriteSet};

/// `POST /guilds/{id}/roles/{role_id}/duplicate`: a copy named "<name> (copy)" that sits just
/// below the source. The source and every role above it move up one position.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    role_id: Snowflake,
    audit: AuditContext<'_>,
) -> AppResult<Role> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_ROLES)?;
    let source = ctx.role(role_id)?.clone();
    if source.is_everyone() {
        return Err(AppError::invalid(
            "role_id",
            "ROLE_EVERYONE_DUPLICATE",
            "The @everyone role cannot be duplicated.",
        ));
    }
    ctx.require_manageable_role(&source)?;
    ctx.require_grantable(source.permissions)?;
    if ctx.roles.len() >= MAX_ROLES_PER_GUILD {
        return Err(AppError::BadRequest {
            code: 30005,
            message: "Maximum number of guild roles reached (250)".into(),
        });
    }
    let copy = source.duplicate(ports.ids.next_id());

    let mut writes = WriteSet::new();
    for mut other in ctx
        .roles
        .iter()
        .filter(|r| !r.is_everyone() && r.position >= source.position)
        .cloned()
    {
        other.position += 1;
        writes
            .mutate(Mutation::UpdateRole(other.clone()))
            .emit(DomainEvent::RoleUpdate(other));
    }
    let changes = vec![
        AuditChange::new("name", None, Some(json!(copy.name))),
        AuditChange::new(
            "permissions",
            None,
            Some(json!(copy.permissions.bits().to_string())),
        ),
        AuditChange::new("color", None, Some(json!(copy.color))),
        AuditChange::new("hoist", None, Some(json!(copy.hoist))),
        AuditChange::new("mentionable", None, Some(json!(copy.mentionable))),
    ];
    writes
        .mutate(Mutation::InsertRole(copy.clone()))
        .emit(DomainEvent::RoleCreate(copy.clone()))
        .audit(audit.entry(
            ports,
            guild_id,
            AuditAction::RoleCreate,
            Some(copy.id),
            changes,
        )?);
    ports.unit_of_work.commit(writes).await?;
    Ok(copy)
}
