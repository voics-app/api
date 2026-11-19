//! Read scheduled events without exposing events in hidden channels.
use chrono::{DateTime, Utc};
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

#[derive(Default)]
pub struct EventPatch {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<Option<DateTime<Utc>>>,
    pub kind: Option<EventKind>,
    pub status: Option<EventStatus>,
    pub image: Option<Option<ImageUpload>>,
}

async fn manager(ports: &Ports, actor: &Actor, event: &ScheduledEvent) -> AppResult<GuildContext> {
    let context = GuildContext::load(ports, event.guild_id, actor.user_id).await?;
    let permissions = context.permissions();
    if !permissions.contains(Permissions::MANAGE_EVENTS)
        && !(event.creator_id == Some(actor.user_id)
            && permissions.contains(Permissions::CREATE_EVENTS))
    {
        return Err(AppError::missing_permissions());
    }
    Ok(context)
}

pub async fn update(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    event_id: Snowflake,
    patch: EventPatch,
) -> AppResult<ScheduledEvent> {
    let before = get(ports, actor, guild_id, event_id).await?;
    let context = manager(ports, actor, &before).await?;
    let mut after = before.clone();
    if let Some(name) = patch.name {
        after.details.name = name.trim().to_owned();
    }
    if let Some(description) = patch.description {
        after.details.description = description;
    }
    if let Some(start) = patch.start {
        if before.details.status != EventStatus::Scheduled || start <= context.now {
            return Err(AppError::invalid(
                "scheduled_start_time",
                "INVALID_TIME",
                "Only a scheduled event can be moved to a future time.",
            ));
        }
        after.details.start = start;
    }
    if let Some(end) = patch.end {
        after.details.end = end;
    }
    if let Some(kind) = patch.kind {
        if before.details.status != EventStatus::Scheduled {
            return Err(AppError::invalid(
                "entity_type",
                "INVALID_EVENT",
                "Only a scheduled event can change location.",
            ));
        }
        after.details.kind = kind;
    }
    if let Some(status) = patch.status {
        if !before.details.status.can_transition_to(status) {
            return Err(AppError::invalid(
                "status",
                "INVALID_STATUS",
                "Invalid event status transition.",
            ));
        }
        after.details.status = status;
    }
    after.details.validate().map_err(|err| {
        AppError::invalid("event", "INVALID_EVENT", format!("Invalid event: {err:?}"))
    })?;
    if let EventKind::Voice { channel_id } | EventKind::Stage { channel_id } = &after.details.kind {
        let channel = ports
            .channels
            .find(*channel_id)
            .await?
            .ok_or(AppError::NotFound(Resource::Channel))?;
        let expected = if matches!(&after.details.kind, EventKind::Stage { .. }) {
            ChannelType::GuildStageVoice
        } else {
            ChannelType::GuildVoice
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
    if let Some(image) = patch.image {
        after.image = match image {
            Some(image) => {
                Some(store_hashed(ports, &format!("guild-events/{event_id}"), image).await?)
            }
            None => None,
        };
    }
    after.revision = before
        .revision
        .checked_add(1)
        .ok_or_else(|| AppError::internal("event revision exhausted"))?;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpdateScheduledEvent {
            event: after.clone(),
            expected_revision: before.revision,
        })
        .emit(DomainEvent::ScheduledEventUpdate {
            before,
            after: after.clone(),
        });
    ports.unit_of_work.commit(writes).await?;
    Ok(after)
}

pub async fn delete(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    event_id: Snowflake,
) -> AppResult<()> {
    let event = get(ports, actor, guild_id, event_id).await?;
    manager(ports, actor, &event).await?;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::DeleteScheduledEvent {
            guild_id,
            event_id,
            expected_revision: event.revision,
        })
        .emit(DomainEvent::ScheduledEventDelete(event));
    ports.unit_of_work.commit(writes).await
}

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
        revision: 0,
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

pub async fn set_interest(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    event_id: Snowflake,
    interested: bool,
) -> AppResult<()> {
    let event = get(ports, actor, guild_id, event_id).await?;
    if interested
        && (event.details.status != EventStatus::Scheduled
            || event.details.start <= ports.clock.now())
    {
        return Err(AppError::invalid(
            "event",
            "EVENT_NOT_SCHEDULED",
            "Interest is available only for scheduled events.",
        ));
    }
    let channel_id = match event.details.kind {
        EventKind::Voice { channel_id } | EventKind::Stage { channel_id } => Some(channel_id),
        EventKind::External { .. } => None,
    };
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::SetScheduledEventInterest {
        guild_id,
        event_id,
        user_id: actor.user_id,
        channel_id,
        interested,
    });
    ports.unit_of_work.commit(writes).await
}
