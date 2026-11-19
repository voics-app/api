use crate::error::ApiError;
use crate::extract::{Body, Caller};
use crate::images;
use crate::routes::ids::snowflake;
use crate::state::AppState;
use application::use_cases::stickers::{self, CreateStickerInput, UpdateStickerInput};
use application::Resource;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use contracts::objects::StickerDto;
use serde::Deserialize;

#[derive(Deserialize)]
struct CreateRequest {
    name: String,
    tags: String,
    image: String,
}
#[derive(Deserialize)]
struct UpdateRequest {
    name: Option<String>,
    tags: Option<String>,
}

async fn list(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(guild): Path<String>,
) -> Result<Json<Vec<StickerDto>>, ApiError> {
    let stickers =
        stickers::list(&state.ports, &actor, snowflake(&guild, Resource::Guild)?).await?;
    Ok(Json(stickers.iter().map(StickerDto::from).collect()))
}
async fn get_one(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path((guild, id)): Path<(String, String)>,
) -> Result<Json<StickerDto>, ApiError> {
    let sticker = stickers::get(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&id, Resource::Sticker)?,
    )
    .await?;
    Ok(Json(StickerDto::from(&sticker)))
}
async fn create(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(guild): Path<String>,
    Body(body): Body<CreateRequest>,
) -> Result<(StatusCode, Json<StickerDto>), ApiError> {
    let input = CreateStickerInput {
        name: body.name,
        tags: body.tags,
        image: images::parse_data_uri(&body.image)?,
    };
    let sticker = stickers::create(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        input,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(StickerDto::from(&sticker))))
}
async fn patch(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path((guild, id)): Path<(String, String)>,
    Body(body): Body<UpdateRequest>,
) -> Result<Json<StickerDto>, ApiError> {
    let input = UpdateStickerInput {
        name: body.name,
        tags: body.tags,
        remove: false,
    };
    let sticker = stickers::update(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&id, Resource::Sticker)?,
        input,
    )
    .await?;
    Ok(Json(StickerDto::from(&sticker)))
}
async fn remove(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path((guild, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let input = UpdateStickerInput {
        name: None,
        tags: None,
        remove: true,
    };
    stickers::update(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&id, Resource::Sticker)?,
        input,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/guilds/{id}/stickers",
            get(list)
                .post(create)
                .layer(DefaultBodyLimit::max(1024 * 1024)),
        )
        .route(
            "/guilds/{id}/stickers/{sticker_id}",
            get(get_one).patch(patch).delete(remove),
        )
}
