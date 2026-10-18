use serde_json::Value;

use domain::application::{
    validate_command, Application, ApplicationCommand, CommandKind, MAX_COMMANDS_PER_SCOPE,
};
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::use_cases::applications::app_access::require_manager;

#[derive(Debug, Clone, Default)]
pub struct CommandInput {
    pub name: Option<String>,
    pub description: Option<String>,
    pub kind: Option<u8>,
    pub options: Option<Value>,
    pub default_member_permissions: Option<Option<Permissions>>,
    pub dm_permission: Option<bool>,
    pub nsfw: Option<bool>,
}

/// Commands may nest at most 25 options per level.
const MAX_OPTIONS: usize = 25;

/// Loads the application and, for guild scope, checks the bot is installed.
pub async fn require_scope(
    ports: &Ports,
    actor: &Actor,
    application_id: Snowflake,
    guild_id: Option<Snowflake>,
) -> AppResult<Application> {
    let application = require_manager(ports, actor, application_id).await?;
    if let Some(guild_id) = guild_id {
        if ports
            .guilds
            .member(guild_id, application.bot_user_id)
            .await?
            .is_none()
        {
            return Err(AppError::missing_access());
        }
    }
    Ok(application)
}

/// Builds a full command from `input`, starting from `base` when updating.
pub fn build(
    ports: &Ports,
    application_id: Snowflake,
    guild_id: Option<Snowflake>,
    base: Option<&ApplicationCommand>,
    input: CommandInput,
) -> AppResult<ApplicationCommand> {
    let kind = match (input.kind, base) {
        (Some(code), _) => CommandKind::from_code(code).ok_or_else(|| {
            AppError::invalid("type", "BASE_TYPE_CHOICES", "Type must be 1, 2 or 3.")
        })?,
        (None, Some(base)) => base.kind,
        (None, None) => CommandKind::ChatInput,
    };
    let name = input
        .name
        .or_else(|| base.map(|b| b.name.clone()))
        .unwrap_or_default();
    let description = input
        .description
        .or_else(|| base.map(|b| b.description.clone()))
        .unwrap_or_default();
    validate_command(kind, &name, &description)?;
    let options = input
        .options
        .or_else(|| base.map(|b| b.options.clone()))
        .unwrap_or(Value::Array(vec![]));
    let valid_options = match &options {
        Value::Array(items) => {
            items.len() <= MAX_OPTIONS && (kind == CommandKind::ChatInput || items.is_empty())
        }
        _ => false,
    };
    if !valid_options {
        return Err(AppError::invalid(
            "options",
            "APPLICATION_COMMAND_OPTIONS_INVALID",
            "Invalid options.",
        ));
    }
    Ok(ApplicationCommand {
        id: base.map(|b| b.id).unwrap_or_else(|| ports.ids.next_id()),
        application_id,
        guild_id,
        name,
        description,
        kind,
        options,
        default_member_permissions: input
            .default_member_permissions
            .unwrap_or_else(|| base.and_then(|b| b.default_member_permissions)),
        dm_permission: input
            .dm_permission
            .or(base.map(|b| b.dm_permission))
            .unwrap_or(true),
        nsfw: input.nsfw.or(base.map(|b| b.nsfw)).unwrap_or(false),
        version: ports.ids.next_id(),
    })
}

pub fn ensure_capacity(existing: &[ApplicationCommand], kind: CommandKind) -> AppResult<()> {
    if existing.iter().filter(|c| c.kind == kind).count() >= MAX_COMMANDS_PER_SCOPE {
        return Err(AppError::BadRequest {
            code: 30032,
            message: "Maximum number of application commands reached".into(),
        });
    }
    Ok(())
}
