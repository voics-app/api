//! Server templates: `/guilds/templates/*` and `/guilds/{id}/templates/*` (api#44).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};

use application::use_cases::templates::{
    self, CreateTemplateInput, ModifyTemplateInput, TemplateView,
};
use application::Resource;
use contracts::objects::{GuildFullDto, GuildTemplateDto};
use contracts::requests::{
    CreateGuildFromTemplateRequest, CreateGuildTemplateRequest, ModifyGuildTemplateRequest,
};

use crate::error::ApiError;
use crate::extract::{Body, Caller};
use crate::images;
use crate::routes::ids::snowflake;
use crate::state::AppState;

fn dto(view: &TemplateView) -> GuildTemplateDto {
    GuildTemplateDto::new(&view.template, view.creator.as_ref())
}

async fn builtin(Caller(_): Caller) -> Json<Vec<GuildTemplateDto>> {
    Json(templates::list_builtin().iter().map(dto).collect())
}

async fn resolve(
    State(state): State<AppState>,
    Caller(_): Caller,
    Path(code): Path<String>,
) -> Result<Json<GuildTemplateDto>, ApiError> {
    Ok(Json(dto(&templates::get(&state.ports, &code).await?)))
}

async fn create_guild(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(code): Path<String>,
    Body(body): Body<CreateGuildFromTemplateRequest>,
) -> Result<(StatusCode, Json<GuildFullDto>), ApiError> {
    let icon = images::optional(body.icon)?;
    let snapshot = templates::create_guild(&state.ports, &actor, &code, body.name, icon).await?;
    let region = &state.ports.settings.default_voice_region;
    Ok((
        StatusCode::CREATED,
        Json(GuildFullDto::new(&snapshot, region)),
    ))
}

async fn list(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
) -> Result<Json<Vec<GuildTemplateDto>>, ApiError> {
    let views = templates::list(&state.ports, &actor, snowflake(&id, Resource::Guild)?).await?;
    Ok(Json(views.iter().map(dto).collect()))
}

async fn create(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
    Body(body): Body<CreateGuildTemplateRequest>,
) -> Result<(StatusCode, Json<GuildTemplateDto>), ApiError> {
    let input = CreateTemplateInput {
        name: body.name,
        description: body.description,
    };
    let view = templates::create(
        &state.ports,
        &actor,
        snowflake(&id, Resource::Guild)?,
        input,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(dto(&view))))
}

async fn sync(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path((id, code)): Path<(String, String)>,
) -> Result<Json<GuildTemplateDto>, ApiError> {
    let guild_id = snowflake(&id, Resource::Guild)?;
    Ok(Json(dto(&templates::sync(
        &state.ports,
        &actor,
        guild_id,
        &code,
    )
    .await?)))
}

async fn modify(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path((id, code)): Path<(String, String)>,
    Body(body): Body<ModifyGuildTemplateRequest>,
) -> Result<Json<GuildTemplateDto>, ApiError> {
    let guild_id = snowflake(&id, Resource::Guild)?;
    let input = ModifyTemplateInput {
        name: body.name,
        description: body.description,
    };
    Ok(Json(dto(&templates::modify(
        &state.ports,
        &actor,
        guild_id,
        &code,
        input,
    )
    .await?)))
}

async fn remove(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path((id, code)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let guild_id = snowflake(&id, Resource::Guild)?;
    templates::delete(&state.ports, &actor, guild_id, &code).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/guilds/templates", get(builtin))
        .route("/guilds/templates/{code}", get(resolve).post(create_guild))
        .route("/guilds/{id}/templates", get(list).post(create))
        .route(
            "/guilds/{id}/templates/{code}",
            axum::routing::put(sync).patch(modify).delete(remove),
        )
}
