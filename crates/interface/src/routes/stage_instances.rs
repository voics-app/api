//! Stage instance HTTP lifecycle.
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::extract::{Body, Caller};
use crate::routes::ids::snowflake;
use crate::state::AppState;
use application::use_cases::stage_instances;
use application::{AppError, Resource};
use contracts::wire::Id;
use domain::stage_instance::StageInstance;

#[derive(Deserialize)]
struct StartRequest {
    channel_id: Id,
    topic: String,
    privacy_level: Option<u8>,
}
#[derive(Deserialize)]
struct UpdateRequest {
    topic: String,
    privacy_level: Option<u8>,
}
#[derive(Serialize)]
struct StageDto {
    id: Id,
    guild_id: Id,
    channel_id: Id,
    topic: String,
    privacy_level: u8,
    discoverable_disabled: bool,
}
impl From<StageInstance> for StageDto {
    fn from(stage: StageInstance) -> Self {
        Self {
            id: Id(stage.id),
            guild_id: Id(stage.guild_id),
            channel_id: Id(stage.channel_id),
            topic: stage.topic,
            privacy_level: 2,
            discoverable_disabled: false,
        }
    }
}
fn check_privacy(privacy: Option<u8>) -> Result<(), AppError> {
    if privacy.is_some_and(|level| level != 2) {
        return Err(AppError::invalid(
            "privacy_level",
            "INVALID_PRIVACY_LEVEL",
            "Only guild-only stages are supported.",
        ));
    }
    Ok(())
}
async fn start(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Body(body): Body<StartRequest>,
) -> Result<(StatusCode, Json<StageDto>), ApiError> {
    check_privacy(body.privacy_level)?;
    Ok((
        StatusCode::CREATED,
        Json(
            stage_instances::start(&state.ports, &actor, body.channel_id.0, body.topic)
                .await?
                .into(),
        ),
    ))
}
async fn fetch(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(channel): Path<String>,
) -> Result<Json<StageDto>, ApiError> {
    Ok(Json(
        stage_instances::get(
            &state.ports,
            &actor,
            snowflake(&channel, Resource::Channel)?,
        )
        .await?
        .into(),
    ))
}
async fn update(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(channel): Path<String>,
    Body(body): Body<UpdateRequest>,
) -> Result<Json<StageDto>, ApiError> {
    check_privacy(body.privacy_level)?;
    Ok(Json(
        stage_instances::update(
            &state.ports,
            &actor,
            snowflake(&channel, Resource::Channel)?,
            body.topic,
        )
        .await?
        .into(),
    ))
}
async fn end(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(channel): Path<String>,
) -> Result<StatusCode, ApiError> {
    stage_instances::end(
        &state.ports,
        &actor,
        snowflake(&channel, Resource::Channel)?,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub fn routes() -> Router<AppState> {
    Router::new().route("/stage-instances", post(start)).route(
        "/stage-instances/{channel_id}",
        get(fetch).patch(update).delete(end),
    )
}
