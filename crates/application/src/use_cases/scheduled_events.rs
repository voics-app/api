//! Read scheduled events without exposing events in hidden channels.
use domain::permissions::Permissions;
use domain::scheduled_event::{EventKind, ScheduledEvent};
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::support::guild_context::GuildContext;

async fn visible(ports: &Ports, context: &GuildContext, event: &ScheduledEvent) -> AppResult<bool> {
    let channel_id = match &event.details.kind {
        EventKind::Voice { channel_id } | EventKind::Stage { channel_id } => *channel_id,
        EventKind::External { .. } => return Ok(true),
    };
    let Some(channel) = ports.channels.find(channel_id).await? else {
        return Ok(false);
    };
    if channel.guild_id != Some(context.guild.id) {
        return Ok(false);
    }
    Ok(context
        .channel_permissions(&channel, None)
        .contains(Permissions::VIEW_CHANNEL))
}

pub async fn list(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
) -> AppResult<Vec<ScheduledEvent>> {
    let context = GuildContext::load(ports, guild_id, actor.user_id).await?;
    let mut result = Vec::new();
    for event in ports.guilds.scheduled_events(guild_id).await? {
        if visible(ports, &context, &event).await? {
            result.push(event);
        }
    }
    Ok(result)
}

pub async fn get(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    event_id: Snowflake,
) -> AppResult<ScheduledEvent> {
    let context = GuildContext::load(ports, guild_id, actor.user_id).await?;
    let event = ports
        .guilds
        .scheduled_event(guild_id, event_id)
        .await?
        .ok_or(AppError::NotFound(Resource::GuildScheduledEvent))?;
    if !visible(ports, &context, &event).await? {
        return Err(AppError::missing_access());
    }
    Ok(event)
}

pub async fn interested(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    event_id: Snowflake,
) -> AppResult<bool> {
    get(ports, actor, guild_id, event_id).await?;
    ports
        .guilds
        .scheduled_event_interested(event_id, actor.user_id)
        .await
}
