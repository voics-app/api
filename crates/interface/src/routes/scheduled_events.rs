//! Read endpoints for scheduled guild events.
use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};

use application::use_cases::scheduled_events;
use application::Resource;
use contracts::wire::timestamp;
use domain::scheduled_event::{EventKind, ScheduledEvent};

use crate::error::ApiError;
use crate::extract::Caller;
use crate::routes::ids::snowflake;
use crate::state::AppState;

fn dto(event: &ScheduledEvent) -> Value {
    let (entity_type, channel_id, metadata) = match &event.details.kind {
        EventKind::Stage { channel_id } => (1, Some(channel_id.to_string()), None),
        EventKind::Voice { channel_id } => (2, Some(channel_id.to_string()), None),
        EventKind::External { location } => (3, None, Some(json!({ "location": location }))),
    };
    json!({
        "id": event.id.to_string(), "guild_id": event.guild_id.to_string(),
        "creator_id": event.creator_id.map(|id| id.to_string()), "channel_id": channel_id,
        "name": event.details.name, "description": event.details.description,
        "scheduled_start_time": timestamp(event.details.start),
        "scheduled_end_time": event.details.end.map(timestamp),
        "status": event.details.status.code(), "entity_type": entity_type,
        "entity_metadata": metadata, "image": event.image, "user_count": event.user_count,
    })
}

async fn list(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(guild): Path<String>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let guild_id = snowflake(&guild, Resource::Guild)?;
    let events = scheduled_events::list(&state.ports, &actor, guild_id).await?;
    Ok(Json(events.iter().map(dto).collect()))
}

async fn get_one(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path((guild, event)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let guild_id = snowflake(&guild, Resource::Guild)?;
    let event_id = snowflake(&event, Resource::GuildScheduledEvent)?;
    Ok(Json(dto(&scheduled_events::get(
        &state.ports,
        &actor,
        guild_id,
        event_id,
    )
    .await?)))
}

async fn interest(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path((guild, event)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let guild_id = snowflake(&guild, Resource::Guild)?;
    let event_id = snowflake(&event, Resource::GuildScheduledEvent)?;
    let interested = scheduled_events::interested(&state.ports, &actor, guild_id, event_id).await?;
    Ok(Json(json!({ "interested": interested })))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/guilds/{id}/scheduled-events", get(list))
        .route("/guilds/{id}/scheduled-events/{event}", get(get_one))
        .route(
            "/guilds/{id}/scheduled-events/{event}/users/@me",
            get(interest),
        )
}
