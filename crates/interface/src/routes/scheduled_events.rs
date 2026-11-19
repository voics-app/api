//! Read endpoints for scheduled guild events.
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};

use application::use_cases::scheduled_events;
use application::Resource;
use contracts::objects::scheduled_event as dto;
use contracts::wire::Id;
use domain::scheduled_event::{EventKind, EventStatus, ScheduledEventDetails};

use crate::error::ApiError;
use crate::extract::{Body, Caller};
use crate::images;
use crate::routes::ids::snowflake;
use crate::state::AppState;

#[derive(Deserialize)]
struct Metadata {
    location: String,
}

#[derive(Deserialize)]
struct CreateRequest {
    name: String,
    description: Option<String>,
    entity_type: u8,
    channel_id: Option<Id>,
    entity_metadata: Option<Metadata>,
    scheduled_start_time: DateTime<Utc>,
    scheduled_end_time: Option<DateTime<Utc>>,
    image: Option<String>,
}

async fn create(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(guild): Path<String>,
    Body(body): Body<CreateRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let kind = match (body.entity_type, body.channel_id, body.entity_metadata) {
        (1, Some(channel), None) => EventKind::Stage {
            channel_id: channel.0,
        },
        (2, Some(channel), None) => EventKind::Voice {
            channel_id: channel.0,
        },
        (3, None, Some(metadata)) => EventKind::External {
            location: metadata.location,
        },
        _ => {
            return Err(application::AppError::invalid(
                "entity_type",
                "INVALID_EVENT",
                "Choose a matching channel or external location.",
            )
            .into())
        }
    };
    let details = ScheduledEventDetails {
        name: body.name,
        description: body.description,
        start: body.scheduled_start_time,
        end: body.scheduled_end_time,
        kind,
        status: EventStatus::Scheduled,
    };
    let event = scheduled_events::create(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        details,
        images::optional(body.image)?,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(dto(&event))))
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

async fn add_interest(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path((guild, event)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    scheduled_events::set_interest(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&event, Resource::GuildScheduledEvent)?,
        true,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_interest(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path((guild, event)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    scheduled_events::set_interest(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&event, Resource::GuildScheduledEvent)?,
        false,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/guilds/{id}/scheduled-events",
            get(list)
                .post(create)
                .layer(DefaultBodyLimit::max(images::IMAGE_BODY_LIMIT)),
        )
        .route("/guilds/{id}/scheduled-events/{event}", get(get_one))
        .route(
            "/guilds/{id}/scheduled-events/{event}/users/@me",
            get(interest).put(add_interest).delete(remove_interest),
        )
}
