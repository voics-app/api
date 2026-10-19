//! Guild emojis (P9).

use axum::extract::DefaultBodyLimit;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};

use application::support::audit::AuditContext;
use application::use_cases::emojis::create_emoji::CreateEmojiInput;
use application::use_cases::emojis::update_emoji::UpdateEmojiInput;
use application::use_cases::emojis::{
    create_emoji, delete_emoji, get_emoji, list_emojis, update_emoji,
};
use application::Resource;
use contracts::objects::EmojiDto;
use contracts::requests::{CreateEmojiRequest, ModifyEmojiRequest};
use contracts::wire::Id;

use crate::error::ApiError;
use crate::extract::{AuditReason, Body, Caller};
use crate::images::{self, IMAGE_BODY_LIMIT};
use crate::routes::ids::snowflake;
use crate::state::AppState;

async fn list(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(guild): Path<String>,
) -> Result<Json<Vec<EmojiDto>>, ApiError> {
    let emojis =
        list_emojis::execute(&state.ports, &actor, snowflake(&guild, Resource::Guild)?).await?;
    Ok(Json(emojis.iter().map(EmojiDto::from).collect()))
}

async fn fetch(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path((guild, emoji)): Path<(String, String)>,
) -> Result<Json<EmojiDto>, ApiError> {
    let found = get_emoji::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&emoji, Resource::Emoji)?,
    )
    .await?;
    Ok(Json(EmojiDto::from(&found)))
}

async fn create(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path(guild): Path<String>,
    Body(body): Body<CreateEmojiRequest>,
) -> Result<(StatusCode, Json<EmojiDto>), ApiError> {
    let input = CreateEmojiInput {
        name: body.name,
        image: images::parse_data_uri(&body.image)?,
        roles: body.roles.into_iter().map(|Id(id)| id).collect(),
    };
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    let created = create_emoji::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        input,
        audit,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(EmojiDto::from(&created))))
}

async fn modify(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path((guild, emoji)): Path<(String, String)>,
    Body(body): Body<ModifyEmojiRequest>,
) -> Result<Json<EmojiDto>, ApiError> {
    let input = UpdateEmojiInput {
        name: body.name,
        roles: body
            .roles
            .map(|roles| roles.into_iter().map(|Id(id)| id).collect()),
    };
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    let updated = update_emoji::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&emoji, Resource::Emoji)?,
        input,
        audit,
    )
    .await?;
    Ok(Json(EmojiDto::from(&updated)))
}

async fn remove(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path((guild, emoji)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    delete_emoji::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&emoji, Resource::Emoji)?,
        audit,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/guilds/{id}/emojis",
            get(list)
                .post(create)
                .layer(DefaultBodyLimit::max(IMAGE_BODY_LIMIT)),
        )
        .route(
            "/guilds/{id}/emojis/{emoji_id}",
            get(fetch)
                .patch(modify)
                .delete(remove)
                .layer(DefaultBodyLimit::max(IMAGE_BODY_LIMIT)),
        )
}
