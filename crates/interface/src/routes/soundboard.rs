use crate::error::ApiError;
use crate::extract::{AuditReason, Body, Caller};
use crate::routes::ids::snowflake;
use crate::state::AppState;
use application::support::audit::AuditContext;
use application::use_cases::soundboard;
use application::Resource;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use base64::Engine;
use domain::soundboard::SoundboardSound;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct Metadata {
    id: String,
    guild_id: String,
    creator_id: Option<String>,
    name: String,
    volume: f32,
}
impl From<&SoundboardSound> for Metadata {
    fn from(sound: &SoundboardSound) -> Self {
        Self {
            id: sound.id.to_string(),
            guild_id: sound.guild_id.to_string(),
            creator_id: sound.creator_id.map(|id| id.to_string()),
            name: sound.name.clone(),
            volume: sound.volume,
        }
    }
}
#[derive(Deserialize)]
struct CreateSound {
    name: String,
    #[serde(default = "one")]
    volume: f32,
    sound: String,
}
fn one() -> f32 {
    1.0
}
#[derive(Deserialize)]
struct ModifySound {
    name: Option<String>,
    volume: Option<f32>,
}
async fn list(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(guild): Path<String>,
) -> Result<Json<Vec<Metadata>>, ApiError> {
    let sounds =
        soundboard::list(&state.ports, &actor, snowflake(&guild, Resource::Guild)?).await?;
    Ok(Json(sounds.iter().map(Metadata::from).collect()))
}
async fn fetch(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path((guild, sound)): Path<(String, String)>,
) -> Result<Json<Metadata>, ApiError> {
    let found = soundboard::get(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&sound, Resource::SoundboardSound)?,
    )
    .await?;
    Ok(Json(Metadata::from(&found)))
}
async fn audio(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path((guild, sound)): Path<(String, String)>,
) -> Result<(HeaderMap, Vec<u8>), ApiError> {
    let found = soundboard::get(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&sound, Resource::SoundboardSound)?,
    )
    .await?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/wav"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    Ok((headers, found.audio))
}
async fn create(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path(guild): Path<String>,
    Body(body): Body<CreateSound>,
) -> Result<(StatusCode, Json<Metadata>), ApiError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body.sound)
        .map_err(|_| application::AppError::invalid("sound", "INVALID", "Expected base64 audio"))?;
    let sound = soundboard::create(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        body.name,
        body.volume,
        bytes,
        AuditContext {
            actor_id: actor.user_id,
            reason: reason.as_deref(),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(Metadata::from(&sound))))
}
async fn modify(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path((guild, sound)): Path<(String, String)>,
    Body(body): Body<ModifySound>,
) -> Result<Json<Metadata>, ApiError> {
    let found = soundboard::update(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&sound, Resource::SoundboardSound)?,
        body.name,
        body.volume,
        AuditContext {
            actor_id: actor.user_id,
            reason: reason.as_deref(),
        },
    )
    .await?;
    Ok(Json(Metadata::from(&found)))
}
async fn delete(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path((guild, sound)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    soundboard::delete(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&sound, Resource::SoundboardSound)?,
        AuditContext {
            actor_id: actor.user_id,
            reason: reason.as_deref(),
        },
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/guilds/{id}/soundboard-sounds",
            get(list).post(create).layer(DefaultBodyLimit::max(700_000)),
        )
        .route(
            "/guilds/{id}/soundboard-sounds/{sound_id}",
            get(fetch).patch(modify).delete(delete),
        )
        .route(
            "/guilds/{id}/soundboard-sounds/{sound_id}/audio",
            get(audio),
        )
}
