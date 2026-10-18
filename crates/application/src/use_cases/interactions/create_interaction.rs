use serde_json::Value;

use domain::application::{Interaction, InteractionKind, InteractionState};
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::events::DomainEvent;
use crate::support::channel_access::{ChannelAccess, PRIVATE_CHANNEL_PERMISSIONS};
use crate::support::guild_context::GuildContext;
use crate::support::views::{member_profile, require_user};
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone)]
pub struct CreateInteractionInput {
    pub application_id: Snowflake,
    pub kind: u8,
    pub channel_id: Snowflake,
    pub data: Value,
    /// The message a component belongs to (component interactions).
    pub source_message_id: Option<Snowflake>,
}

/// A user invokes a command, component, autocomplete or modal; the owning
/// bot receives INTERACTION_CREATE.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    input: CreateInteractionInput,
) -> AppResult<Snowflake> {
    actor.require_user()?;
    let kind = InteractionKind::from_code(input.kind)
        .filter(|k| *k != InteractionKind::Ping)
        .ok_or_else(|| {
            AppError::invalid("type", "BASE_TYPE_CHOICES", "Type must be between 2 and 5.")
        })?;
    let application = ports
        .applications
        .find(input.application_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Application))?;
    let access = ChannelAccess::visible(ports, input.channel_id, actor.user_id).await?;
    access.require(Permissions::USE_APPLICATION_COMMANDS)?;

    let bot_present = match access.channel.guild_id {
        Some(guild_id) => ports
            .guilds
            .member(guild_id, application.bot_user_id)
            .await?
            .is_some(),
        None => access.channel.is_recipient(application.bot_user_id),
    };
    if !bot_present {
        return Err(AppError::missing_access());
    }
    if kind == InteractionKind::ApplicationCommand {
        check_command(ports, &access, input.application_id, &input.data).await?;
    }

    let app_permissions = match &access.guild {
        Some(guild) => {
            let bot = GuildContext::load(ports, guild.guild.id, application.bot_user_id).await?;
            bot.channel_permissions(&access.channel, access.parent.as_ref())
        }
        None => PRIVATE_CHANNEL_PERMISSIONS,
    };
    let member = match &access.guild {
        Some(ctx) => Some(Box::new(member_profile(ports, ctx.member.clone()).await?)),
        None => None,
    };
    let user = require_user(ports, actor.user_id).await?;
    let token = ports.secrets.token(32);
    let interaction = Interaction {
        id: ports.ids.next_id(),
        application_id: application.id,
        kind,
        guild_id: access.channel.guild_id,
        channel_id: input.channel_id,
        user_id: actor.user_id,
        token_hash: ports.secrets.hash(&token),
        data: input.data,
        state: InteractionState::Pending,
        original_message_id: None,
        source_message_id: input.source_message_id,
        created_at: ports.clock.now(),
    };
    let id = interaction.id;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::InsertInteraction(interaction.clone()))
        .emit(DomainEvent::InteractionCreate {
            application_id: application.id,
            bot_user_id: application.bot_user_id,
            interaction,
            token,
            app_permissions,
            user,
            member,
        });
    ports.unit_of_work.commit(writes).await?;
    Ok(id)
}

/// The command must exist, be usable in this scope and respect its default permissions.
async fn check_command(
    ports: &Ports,
    access: &ChannelAccess,
    application_id: Snowflake,
    data: &Value,
) -> AppResult<()> {
    let command_id: Snowflake = data
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| {
            AppError::invalid("data.id", "BASE_TYPE_REQUIRED", "A command id is required.")
        })?;
    let command = ports
        .applications
        .command(application_id, command_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Command))?;
    if command.guild_id.is_some() && command.guild_id != access.channel.guild_id {
        return Err(AppError::NotFound(Resource::Command));
    }
    match &access.guild {
        Some(ctx) => {
            if let Some(required) = command.default_member_permissions {
                ctx.require(required)?;
            }
        }
        None if !command.dm_permission => return Err(AppError::missing_access()),
        None => {}
    }
    Ok(())
}
