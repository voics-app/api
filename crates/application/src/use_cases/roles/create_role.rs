use serde_json::json;

use domain::audit::{AuditAction, AuditChange};
use domain::guild::Role;
use domain::permissions::Permissions;
use domain::validation;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::guild_context::GuildContext;
use crate::write_set::{Mutation, WriteSet};

pub const MAX_ROLES_PER_GUILD: usize = 250;

#[derive(Debug, Clone, Default)]
pub struct RoleInput {
    pub name: Option<String>,
    pub permissions: Option<Permissions>,
    pub color: Option<u32>,
    pub hoist: Option<bool>,
    pub mentionable: Option<bool>,
    pub unicode_emoji: Option<Option<String>>,
}

/// Applies the editable fields of a role, enforcing the grant rule.
pub fn apply_role_input(ctx: &GuildContext, role: &mut Role, input: RoleInput) -> AppResult<()> {
    if let Some(name) = input.name {
        validation::role_name(&name)?;
        role.name = name.trim().to_owned();
    }
    if let Some(permissions) = input.permissions {
        let added = permissions.missing_from(role.permissions);
        ctx.require_grantable(added)?;
        role.permissions = Permissions::from_bits_truncate(permissions.bits());
    }
    if let Some(color) = input.color {
        if color > 0xFF_FFFF {
            return Err(AppError::invalid(
                "color",
                "BASE_TYPE_BAD_INT",
                "Color must be a 24-bit RGB value.",
            ));
        }
        role.color = color;
    }
    if let Some(hoist) = input.hoist {
        role.hoist = hoist;
    }
    if let Some(mentionable) = input.mentionable {
        role.mentionable = mentionable;
    }
    if let Some(emoji) = input.unicode_emoji {
        role.unicode_emoji = emoji;
    }
    Ok(())
}

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    input: RoleInput,
    audit: AuditContext<'_>,
) -> AppResult<Role> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_ROLES)?;
    if ctx.roles.len() >= MAX_ROLES_PER_GUILD {
        return Err(AppError::BadRequest {
            code: 30005,
            message: "Maximum number of guild roles reached (250)".into(),
        });
    }
    let mut role = Role {
        id: ports.ids.next_id()?,
        name: "new role".into(),
        position: 1,
        ..Role::everyone(guild_id, Permissions::NONE)
    };
    apply_role_input(
        &ctx,
        &mut role,
        RoleInput {
            name: input.name.clone().or(Some("new role".into())),
            ..input
        },
    )?;

    let mut writes = WriteSet::new();
    for mut other in ctx.roles.iter().filter(|r| !r.is_everyone()).cloned() {
        other.position += 1;
        writes
            .mutate(Mutation::UpdateRole(other.clone()))
            .emit(DomainEvent::RoleUpdate(other));
    }
    let changes = vec![
        AuditChange::new("name", None, Some(json!(role.name))),
        AuditChange::new(
            "permissions",
            None,
            Some(json!(role.permissions.bits().to_string())),
        ),
    ];
    writes
        .mutate(Mutation::InsertRole(role.clone()))
        .emit(DomainEvent::RoleCreate(role.clone()))
        .audit(audit.entry(
            ports,
            guild_id,
            AuditAction::RoleCreate,
            Some(role.id),
            changes,
        )?);
    ports.unit_of_work.commit(writes).await?;
    Ok(role)
}
