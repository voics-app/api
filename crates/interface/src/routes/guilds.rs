//! `/api/v1/guilds`, `/users/@me/guilds` and the guild audit log (P9).

use axum::extract::DefaultBodyLimit;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use application::ports::AuditLogQuery;
use application::support::audit::AuditContext;
use application::use_cases::audit::list_audit_log;
use application::use_cases::guilds::create_guild::CreateGuildInput;
use application::use_cases::guilds::update_guild::UpdateGuildInput;
use application::use_cases::guilds::{
    create_guild, delete_guild, get_guild, leave_guild, list_current_user_guilds, update_guild,
};
use application::{AppError, Resource};
use contracts::objects::{AuditLogDto, GuildDto, GuildFullDto};
use contracts::requests::{CreateGuildRequest, ModifyGuildRequest};
use contracts::screening::{AcceptScreeningRequest, ScreeningDto, SetScreeningRequest};
use contracts::wire::Id;
use domain::audit::AuditAction;

use crate::error::ApiError;
use crate::extract::{AuditReason, Body, Caller};
use crate::images::{self, IMAGE_BODY_LIMIT};
use crate::routes::ids::snowflake;
use crate::state::AppState;

fn region(state: &AppState) -> &str {
    &state.ports.settings.default_voice_region
}

fn screening_dto(config: application::ports::ScreeningConfig, pending: bool) -> ScreeningDto {
    ScreeningDto {
        enabled: config.enabled,
        rules: config.rules,
        revision: config.revision,
        pending,
    }
}

async fn get_screening(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
) -> Result<Json<ScreeningDto>, ApiError> {
    let (config, pending) = application::use_cases::guilds::screening::get(
        &state.ports,
        &actor,
        snowflake(&id, Resource::Guild)?,
    )
    .await?;
    Ok(Json(screening_dto(config, pending)))
}

async fn put_screening(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
    Body(body): Body<SetScreeningRequest>,
) -> Result<Json<ScreeningDto>, ApiError> {
    Ok(Json(screening_dto(
        application::use_cases::guilds::screening::set(
            &state.ports,
            &actor,
            snowflake(&id, Resource::Guild)?,
            body.enabled,
            body.rules,
        )
        .await?,
        false,
    )))
}

async fn accept_screening(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
    Body(body): Body<AcceptScreeningRequest>,
) -> Result<StatusCode, ApiError> {
    application::use_cases::guilds::screening::accept(
        &state.ports,
        &actor,
        snowflake(&id, Resource::Guild)?,
        body.revision,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_welcome(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        application::use_cases::guilds::welcome::get(
            &state.ports,
            &actor,
            snowflake(&id, Resource::Guild)?,
        )
        .await?,
    ))
}

async fn put_welcome(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
    Body(body): Body<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        application::use_cases::guilds::welcome::set(
            &state.ports,
            &actor,
            snowflake(&id, Resource::Guild)?,
            body,
        )
        .await?,
    ))
}

async fn create(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Body(body): Body<CreateGuildRequest>,
) -> Result<(StatusCode, Json<GuildFullDto>), ApiError> {
    let input = CreateGuildInput {
        name: body.name,
        icon: images::optional(body.icon)?,
        voice_region: body.voice_region,
        template: None,
    };
    let snapshot = create_guild::execute(&state.ports, &actor, input).await?;
    Ok((
        StatusCode::CREATED,
        Json(GuildFullDto::new(&snapshot, region(&state))),
    ))
}

async fn fetch(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
) -> Result<Json<GuildFullDto>, ApiError> {
    let snapshot =
        get_guild::execute(&state.ports, &actor, snowflake(&id, Resource::Guild)?).await?;
    Ok(Json(GuildFullDto::new(&snapshot, region(&state))))
}

async fn modify(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path(id): Path<String>,
    Body(body): Body<ModifyGuildRequest>,
) -> Result<Json<GuildDto>, ApiError> {
    let input = UpdateGuildInput {
        features: body.features,
        rules_channel_id: body.rules_channel_id.map(|id| id.map(|id| id.0)),
        public_updates_channel_id: body.public_updates_channel_id.map(|id| id.map(|id| id.0)),
        name: body.name,
        icon: images::patch(body.icon)?,
        description: body.description,
        voice_region: body.voice_region,
        verification_level: body.verification_level,
        default_message_notifications: body.default_message_notifications,
        explicit_content_filter: body.explicit_content_filter,
        afk_channel_id: body.afk_channel_id.map(|inner| inner.map(|Id(id)| id)),
        afk_timeout: body.afk_timeout,
        system_channel_id: body.system_channel_id.map(|inner| inner.map(|Id(id)| id)),
        system_channel_flags: body.system_channel_flags,
        banner: images::patch(body.banner)?,
        splash: images::patch(body.splash)?,
        owner_id: body.owner_id.map(|Id(id)| id),
        prune_admin_only: body.prune_admin_only,
        audit_log_enabled: body.audit_log_enabled,
        audit_log_retention_days: body.audit_log_retention_days,
    };
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    let guild_id = snowflake(&id, Resource::Guild)?;
    let guild = update_guild::execute(&state.ports, &actor, guild_id, input, audit).await?;
    Ok(Json(GuildDto::new(&guild, &[], &[], region(&state))))
}

async fn remove(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    delete_guild::execute(&state.ports, &actor, snowflake(&id, Resource::Guild)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn my_guilds(
    State(state): State<AppState>,
    Caller(actor): Caller,
) -> Result<Json<Vec<GuildDto>>, ApiError> {
    let guilds = list_current_user_guilds::execute(&state.ports, &actor).await?;
    let region = region(&state);
    Ok(Json(
        guilds
            .iter()
            .map(|(guild, permissions)| GuildDto::partial(guild, *permissions, region))
            .collect(),
    ))
}

async fn leave(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    leave_guild::execute(&state.ports, &actor, snowflake(&id, Resource::Guild)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct AuditQuery {
    user_id: Option<String>,
    target_id: Option<String>,
    action_type: Option<u16>,
    before: Option<String>,
    limit: Option<u32>,
}

async fn audit_logs(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
    Query(params): Query<AuditQuery>,
) -> Result<Json<AuditLogDto>, ApiError> {
    let action = match params.action_type {
        Some(code) => Some(AuditAction::from_code(code).ok_or_else(|| {
            AppError::invalid(
                "action_type",
                "INVALID_AUDIT_ACTION",
                "Unknown audit action.",
            )
        })?),
        None => None,
    };
    let query = AuditLogQuery {
        user_id: params
            .user_id
            .as_deref()
            .map(|raw| snowflake(raw, Resource::User))
            .transpose()?,
        target_id: params
            .target_id
            .as_deref()
            .map(|raw| snowflake(raw, Resource::User))
            .transpose()?,
        action,
        before: params
            .before
            .as_deref()
            .map(|raw| snowflake(raw, Resource::Message))
            .transpose()?,
        limit: params.limit.unwrap_or(50).clamp(1, 100),
    };
    let page = list_audit_log::execute(
        &state.ports,
        &actor,
        snowflake(&id, Resource::Guild)?,
        query,
    )
    .await?;
    Ok(Json(AuditLogDto::new(&page.entries, &page.users)))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/guilds/{id}/member-verification",
            get(get_screening).put(put_screening),
        )
        .route(
            "/guilds/{id}/member-verification/accept",
            post(accept_screening),
        )
        .route(
            "/guilds/{id}/welcome-screen",
            get(get_welcome).put(put_welcome),
        )
        .route("/guilds", post(create))
        .route(
            "/guilds/{id}",
            get(fetch)
                .patch(modify)
                .delete(remove)
                .layer(DefaultBodyLimit::max(IMAGE_BODY_LIMIT)),
        )
        .route("/guilds/{id}/audit-logs", get(audit_logs))
        .route("/users/@me/guilds", get(my_guilds))
        .route("/users/@me/guilds/{id}", axum::routing::delete(leave))
}
