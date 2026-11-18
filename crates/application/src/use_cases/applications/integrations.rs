use super::app_access::{view, ApplicationView};
use crate::{
    context::{Actor, Ports},
    error::{AppError, AppResult, Resource},
    events::DomainEvent,
    support::{audit::AuditContext, guild_context::GuildContext, views::require_user},
    write_set::{Mutation, WriteSet},
};
use chrono::{DateTime, Utc};
use domain::{
    audit::{AuditAction, AuditChange},
    permissions::Permissions,
    Snowflake,
};
use serde_json::json;

pub struct IntegrationView {
    pub application: ApplicationView,
    pub permissions: Permissions,
    pub installed_at: DateTime<Utc>,
}

pub async fn list(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
) -> AppResult<Vec<IntegrationView>> {
    actor.require_user()?;
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_GUILD)?;
    let mut result = Vec::new();
    for application in ports.applications.installed_in_guild(guild_id).await? {
        let Some(member) = ports
            .guilds
            .member(guild_id, application.bot_user_id)
            .await?
        else {
            continue;
        };
        result.push(IntegrationView {
            permissions: ctx.access().base(&member),
            installed_at: member.joined_at,
            application: view(ports, application).await?,
        });
    }
    Ok(result)
}

pub async fn remove(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    app: Snowflake,
    audit: AuditContext<'_>,
) -> AppResult<()> {
    actor.require_user()?;
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_GUILD | Permissions::KICK_MEMBERS)?;
    let installation = ports
        .applications
        .installation(guild_id, app)
        .await?
        .ok_or(AppError::NotFound(Resource::Application))?;
    let bot_id = installation.bot_user_id;
    let member = ports
        .guilds
        .member(guild_id, bot_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Member))?;
    ctx.require_outranks(&member)?;
    let user = require_user(ports, bot_id).await?;
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::GuardApplicationInstallation {
        guild_id,
        application_id: app,
        bot_user_id: bot_id,
        expected_generation: Some(installation.generation),
        expected_revision: Some(installation.generation),
    });
    for role in ctx
        .roles
        .iter()
        .filter(|role| role.managed && role.bot_id == Some(bot_id))
    {
        writes
            .mutate(Mutation::DeleteRole {
                guild_id,
                role_id: role.id,
            })
            .emit(DomainEvent::RoleDelete {
                guild_id,
                role_id: role.id,
            });
    }
    writes
        .mutate(Mutation::DeleteMember {
            guild_id,
            user_id: bot_id,
        })
        .emit(DomainEvent::MemberRemove { guild_id, user })
        .emit(DomainEvent::GuildDelete {
            guild_id,
            recipients: Some(vec![bot_id]),
        })
        .audit(audit.entry(
            ports,
            guild_id,
            AuditAction::MemberKick,
            Some(bot_id),
            vec![AuditChange::new(
                "application_id",
                Some(json!(app.to_string())),
                None,
            )],
        )?);
    ports.unit_of_work.commit(writes).await
}
