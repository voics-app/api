//! Read scheduled events without exposing events in hidden channels.
use domain::channel::ChannelType;
use domain::permissions::Permissions;
use domain::scheduled_event::{EventKind, EventStatus, ScheduledEvent, ScheduledEventDetails};
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::events::DomainEvent;
use crate::support::guild_context::GuildContext;
use crate::support::images::{store_hashed, ImageUpload};
use crate::write_set::{Mutation, WriteSet};

pub async fn create(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    mut details: ScheduledEventDetails,
    image: Option<ImageUpload>,
) -> AppResult<ScheduledEvent> {
    let context = GuildContext::load(ports, guild_id, actor.user_id).await?;
    if !context
        .permissions()
        .intersects(Permissions::CREATE_EVENTS | Permissions::MANAGE_EVENTS)
    {
        return Err(AppError::missing_permissions());
    }
    details.name = details.name.trim().to_owned();
    details.status = EventStatus::Scheduled;
    details.validate().map_err(|err| {
        AppError::invalid("event", "INVALID_EVENT", format!("Invalid event: {err:?}"))
    })?;
    if details.start <= context.now {
        return Err(AppError::invalid(
            "scheduled_start_time",
            "INVALID_TIME",
            "Start must be in the future.",
        ));
    }
    match &details.kind {
        EventKind::Voice { channel_id } | EventKind::Stage { channel_id } => {
            let channel = ports
                .channels
                .find(*channel_id)
                .await?
                .ok_or(AppError::NotFound(Resource::Channel))?;
            let expected = match &details.kind {
                EventKind::Stage { .. } => ChannelType::GuildStageVoice,
                _ => ChannelType::GuildVoice,
            };
            if channel.guild_id != Some(guild_id) || channel.kind != expected {
                return Err(AppError::invalid(
                    "channel_id",
                    "INVALID_CHANNEL",
                    "Select a matching channel in this server.",
                ));
            }
            context.require_in_channel(&channel, None, Permissions::VIEW_CHANNEL)?;
        }
        EventKind::External { .. } => {}
    }
    let id = ports.ids.next_id()?;
    let image = match image {
        Some(image) => Some(store_hashed(ports, &format!("guild-events/{id}"), image).await?),
        None => None,
    };
    let event = ScheduledEvent {
        id,
        guild_id,
        creator_id: Some(actor.user_id),
        details,
        image,
        user_count: 0,
    };
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::InsertScheduledEvent(event.clone()))
        .emit(DomainEvent::ScheduledEventCreate(event.clone()));
    ports.unit_of_work.commit(writes).await?;
    Ok(event)
}

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
