use domain::application::{Interaction, Webhook, WebhookKind};
use domain::channel::Channel;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::support::guild_context::GuildContext;

/// Loads a webhook and requires MANAGE_WEBHOOKS in its guild.
pub async fn require_manager(ports: &Ports, actor: &Actor, webhook_id: Snowflake) -> AppResult<(Webhook, GuildContext)> {
    let webhook = ports.webhooks.find(webhook_id).await?.ok_or(AppError::NotFound(Resource::Webhook))?;
    let ctx = match GuildContext::load(ports, webhook.guild_id, actor.user_id).await {
        Ok(ctx) => ctx,
        Err(AppError::Forbidden(_)) => return Err(AppError::NotFound(Resource::Webhook)),
        Err(other) => return Err(other),
    };
    ctx.require(Permissions::MANAGE_WEBHOOKS)?;
    Ok((webhook, ctx))
}

/// Channels that can host webhooks: guild channels with messages.
pub fn require_hostable(channel: &Channel) -> AppResult<()> {
    use domain::channel::ChannelType::*;
    if channel.guild_id.is_some() && matches!(channel.kind, GuildText | GuildAnnouncement | GuildVoice | GuildStageVoice | GuildForum)
    {
        Ok(())
    } else {
        Err(AppError::BadRequest { code: 50024, message: "Cannot execute action on this channel type".into() })
    }
}

/// Resolves an incoming webhook from its id and token.
pub async fn incoming(ports: &Ports, webhook_id: Snowflake, token: &str) -> AppResult<Option<Webhook>> {
    let Some(webhook) = ports.webhooks.find(webhook_id).await? else {
        return Ok(None);
    };
    if webhook.kind != WebhookKind::Incoming || webhook.token_hash != ports.secrets.hash(token) {
        return Err(AppError::NotFound(Resource::Webhook));
    }
    Ok(Some(webhook))
}

/// Resolves a live, acknowledged interaction from the application id and token.
pub async fn followup_interaction(ports: &Ports, application_id: Snowflake, token: &str) -> AppResult<Interaction> {
    let interaction = ports
        .interactions
        .find_by_token(application_id, &ports.secrets.hash(token))
        .await?
        .ok_or(AppError::NotFound(Resource::Interaction))?;
    interaction.ensure_followup_allowed(ports.clock.now())?;
    Ok(interaction)
}
