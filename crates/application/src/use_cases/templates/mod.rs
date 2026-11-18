//! Server templates: share a guild's structure and create guilds from it (api#44).

use domain::permissions::Permissions;
use domain::template::{self, GuildTemplate};
use domain::user::User;
use domain::validation;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::support::guild_context::GuildContext;
use crate::support::images::ImageUpload;
use crate::use_cases::guilds::create_guild::{self, CreateGuildInput};
use domain::guild::GuildSnapshot;

/// A template with its creator resolved for the response.
#[derive(Debug, Clone)]
pub struct TemplateView {
    pub template: GuildTemplate,
    pub creator: Option<User>,
    pub is_dirty: bool,
}

#[derive(Debug, Clone)]
pub struct CreateTemplateInput {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ModifyTemplateInput {
    pub name: Option<String>,
    /// `Some(None)` clears the description.
    pub description: Option<Option<String>>,
}

async fn view(ports: &Ports, template: GuildTemplate) -> AppResult<TemplateView> {
    let creator = match template.creator_id {
        Some(id) => ports.users.find(id).await?,
        None => None,
    };
    let is_dirty = match template.source_guild_id {
        Some(guild_id) => current_snapshot(ports, guild_id).await? != template.source,
        None => false,
    };
    Ok(TemplateView {
        template,
        creator,
        is_dirty,
    })
}

async fn current_snapshot(
    ports: &Ports,
    guild_id: Snowflake,
) -> AppResult<template::TemplateSource> {
    let guild = ports
        .guilds
        .find(guild_id)
        .await?
        .ok_or(AppError::NotFound(Resource::GuildTemplate))?;
    let roles = ports.guilds.roles(guild_id).await?;
    let channels = ports.channels.list_for_guild(guild_id).await?;
    Ok(template::capture(&guild, &roles, &channels))
}

fn clean_description(description: Option<String>) -> AppResult<Option<String>> {
    let description = description
        .map(|d| d.trim().to_owned())
        .filter(|d| !d.is_empty());
    if let Some(text) = &description {
        validation::template_description(text)?;
    }
    Ok(description)
}

/// `GET /guilds/templates`: the built-in templates.
pub fn list_builtin() -> Vec<TemplateView> {
    template::builtins()
        .into_iter()
        .map(|template| TemplateView {
            template,
            creator: None,
            is_dirty: false,
        })
        .collect()
}

/// `GET /guilds/templates/{code}`: possession of the code grants preview access.
pub async fn get(ports: &Ports, code: &str) -> AppResult<TemplateView> {
    view(ports, resolve(ports, code).await?).await
}

async fn resolve(ports: &Ports, code: &str) -> AppResult<GuildTemplate> {
    if template::is_builtin_code(code) {
        return template::builtin(code).ok_or(AppError::NotFound(Resource::GuildTemplate));
    }
    ports
        .guilds
        .template(code)
        .await?
        .ok_or(AppError::NotFound(Resource::GuildTemplate))
}

async fn manager(ports: &Ports, actor: &Actor, guild_id: Snowflake) -> AppResult<GuildContext> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_GUILD)?;
    Ok(ctx)
}

/// The template `code` if it belongs to the guild.
async fn owned(ports: &Ports, guild_id: Snowflake, code: &str) -> AppResult<GuildTemplate> {
    ports
        .guilds
        .template(code)
        .await?
        .filter(|t| t.source_guild_id == Some(guild_id))
        .ok_or(AppError::NotFound(Resource::GuildTemplate))
}

async fn snapshot(ports: &Ports, ctx: &GuildContext) -> AppResult<template::TemplateSource> {
    let channels = ports.channels.list_for_guild(ctx.guild.id).await?;
    Ok(template::capture(&ctx.guild, &ctx.roles, &channels))
}

/// `GET /guilds/{id}/templates`; needs MANAGE_GUILD.
pub async fn list(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
) -> AppResult<Vec<TemplateView>> {
    manager(ports, actor, guild_id).await?;
    let mut out = Vec::new();
    for template in ports.guilds.templates_for_guild(guild_id).await? {
        out.push(view(ports, template).await?);
    }
    Ok(out)
}

/// `POST /guilds/{id}/templates`; a guild has at most one template.
pub async fn create(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    input: CreateTemplateInput,
) -> AppResult<TemplateView> {
    actor.require_user()?;
    let ctx = manager(ports, actor, guild_id).await?;
    validation::template_name(&input.name)?;
    let description = clean_description(input.description)?;
    if !ports.guilds.templates_for_guild(guild_id).await?.is_empty() {
        return Err(AppError::BadRequest {
            code: 30_031,
            message: "Guild already has a template.".into(),
        });
    }
    let now = ports.clock.now();
    let created = GuildTemplate {
        code: ports.secrets.invite_code(),
        name: input.name.trim().to_owned(),
        description,
        usage_count: 0,
        creator_id: Some(actor.user_id),
        source_guild_id: Some(guild_id),
        source: snapshot(ports, &ctx).await?,
        created_at: now,
        updated_at: now,
    };
    ports.guilds.insert_template(&created).await?;
    view(ports, created).await
}

/// `PUT /guilds/{id}/templates/{code}`: re-snapshots the guild.
pub async fn sync(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    code: &str,
) -> AppResult<TemplateView> {
    let ctx = manager(ports, actor, guild_id).await?;
    let mut template = owned(ports, guild_id, code).await?;
    template.source = snapshot(ports, &ctx).await?;
    template.updated_at = ports.clock.now();
    ports.guilds.update_template(&template).await?;
    view(ports, template).await
}

/// `PATCH /guilds/{id}/templates/{code}`.
pub async fn modify(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    code: &str,
    input: ModifyTemplateInput,
) -> AppResult<TemplateView> {
    manager(ports, actor, guild_id).await?;
    let mut template = owned(ports, guild_id, code).await?;
    if let Some(name) = input.name {
        validation::template_name(&name)?;
        template.name = name.trim().to_owned();
    }
    if let Some(description) = input.description {
        template.description = clean_description(description)?;
    }
    template.updated_at = ports.clock.now();
    ports.guilds.update_template(&template).await?;
    view(ports, template).await
}

/// `DELETE /guilds/{id}/templates/{code}`.
pub async fn delete(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    code: &str,
) -> AppResult<()> {
    manager(ports, actor, guild_id).await?;
    owned(ports, guild_id, code).await?;
    ports.guilds.delete_template(code).await
}

/// `POST /guilds/templates/{code}`: a new guild owned by the caller.
pub async fn create_guild(
    ports: &Ports,
    actor: &Actor,
    code: &str,
    name: String,
    icon: Option<ImageUpload>,
) -> AppResult<GuildSnapshot> {
    actor.require_user()?;
    let template = resolve(ports, code).await?;
    let snapshot = create_guild::execute(
        ports,
        actor,
        CreateGuildInput {
            name,
            icon,
            voice_region: None,
            template: Some(template.source),
        },
    )
    .await?;
    if !template::is_builtin_code(code) {
        ports.guilds.bump_template_usage(code).await?;
    }
    Ok(snapshot)
}
