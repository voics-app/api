//! Webhooks (P9): channel webhooks, their management, execution and the
//! original-response endpoints of interactions.

use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{header, HeaderName, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::Value;

use application::support::audit::AuditContext;
use application::use_cases::webhooks::create_webhook::{self, CreateWebhookInput};
use application::use_cases::webhooks::update_webhook::{self, UpdateWebhookInput};
use application::use_cases::webhooks::{
    delete_original_response, delete_webhook, edit_original_response, execute_webhook, get_webhook,
    list_guild_webhooks, reset_webhook_token,
};
use application::Resource;
use contracts::objects::WebhookDto;
use contracts::requests::{CreateWebhookRequest, ModifyWebhookRequest};
use contracts::wire::Id;

use crate::error::ApiError;
use crate::extract::{AuditReason, Body, Caller};
use crate::images;
use crate::routes::ids::snowflake;
use crate::state::AppState;

fn token_base(state: &AppState) -> String {
    state.ports.settings.public_api_base.clone()
}

async fn create(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path(channel): Path<String>,
    Body(body): Body<CreateWebhookRequest>,
) -> Result<
    (
        StatusCode,
        [(HeaderName, &'static str); 1],
        Json<WebhookDto>,
    ),
    ApiError,
> {
    let input = CreateWebhookInput {
        name: body.name,
        avatar: images::optional(body.avatar)?,
    };
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    let (webhook, token) = create_webhook::execute(
        &state.ports,
        &actor,
        snowflake(&channel, Resource::Channel)?,
        input,
        audit,
    )
    .await?;
    let base = token_base(&state);
    Ok((
        StatusCode::CREATED,
        [(header::CACHE_CONTROL, "no-store")],
        Json(WebhookDto::new(&webhook, Some((&base, &token)))),
    ))
}

async fn reset_token(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path(id): Path<String>,
) -> Result<([(HeaderName, &'static str); 1], Json<WebhookDto>), ApiError> {
    let (webhook, token) = reset_webhook_token::execute(
        &state.ports,
        &actor,
        snowflake(&id, Resource::Webhook)?,
        AuditContext {
            actor_id: actor.user_id,
            reason: reason.as_deref(),
        },
    )
    .await?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(WebhookDto::new(
            &webhook,
            Some((&token_base(&state), &token)),
        )),
    ))
}

async fn guild_webhooks(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(guild): Path<String>,
) -> Result<Json<Vec<WebhookDto>>, ApiError> {
    let found =
        list_guild_webhooks::execute(&state.ports, &actor, snowflake(&guild, Resource::Guild)?)
            .await?;
    Ok(Json(
        found
            .iter()
            .map(|webhook| WebhookDto::new(webhook, None))
            .collect(),
    ))
}

async fn fetch(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
) -> Result<Json<WebhookDto>, ApiError> {
    let webhook =
        get_webhook::execute(&state.ports, &actor, snowflake(&id, Resource::Webhook)?).await?;
    Ok(Json(WebhookDto::new(&webhook, None)))
}

async fn modify(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path(id): Path<String>,
    Body(body): Body<ModifyWebhookRequest>,
) -> Result<Json<WebhookDto>, ApiError> {
    let input = UpdateWebhookInput {
        name: body.name,
        avatar: images::patch(body.avatar)?,
        channel_id: body.channel_id.map(|Id(id)| id),
    };
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    let webhook = update_webhook::execute(
        &state.ports,
        &actor,
        snowflake(&id, Resource::Webhook)?,
        input,
        audit,
    )
    .await?;
    Ok(Json(WebhookDto::new(&webhook, None)))
}

async fn remove(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    delete_webhook::execute(
        &state.ports,
        &actor,
        snowflake(&id, Resource::Webhook)?,
        audit,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /webhooks/{id}/{token}`: executes an incoming webhook, or posts a follow-up for an interaction.
async fn execute(
    State(state): State<AppState>,
    Path((id, token)): Path<(String, String)>,
    Body(body): Body<Value>,
) -> Result<Json<Value>, ApiError> {
    let stored = execute_webhook::execute(
        &state.ports,
        snowflake(&id, Resource::Webhook)?,
        &token,
        body,
    )
    .await?;
    Ok(Json(stored.body))
}

/// `PATCH /webhooks/{app_id}/{token}/messages/@original`.
async fn edit_original(
    State(state): State<AppState>,
    Path((application, token)): Path<(String, String)>,
    Body(body): Body<Value>,
) -> Result<Json<Value>, ApiError> {
    let stored = edit_original_response::execute(
        &state.ports,
        snowflake(&application, Resource::Application)?,
        &token,
        body,
    )
    .await?;
    Ok(Json(stored.body))
}

/// `DELETE /webhooks/{app_id}/{token}/messages/@original`.
async fn delete_original(
    State(state): State<AppState>,
    Path((application, token)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    delete_original_response::execute(
        &state.ports,
        snowflake(&application, Resource::Application)?,
        &token,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/channels/{id}/webhooks",
            post(create).layer(DefaultBodyLimit::max(images::IMAGE_BODY_LIMIT)),
        )
        .route("/guilds/{id}/webhooks", get(guild_webhooks))
        .route(
            "/webhooks/{id}",
            get(fetch)
                .patch(modify)
                .delete(remove)
                .layer(DefaultBodyLimit::max(images::IMAGE_BODY_LIMIT)),
        )
        .route("/webhooks/{id}/reset-token", post(reset_token))
        .route("/webhooks/{id}/{token}", post(execute))
        .route(
            "/webhooks/{app_id}/{token}/messages/@original",
            axum::routing::patch(edit_original).delete(delete_original),
        )
}
