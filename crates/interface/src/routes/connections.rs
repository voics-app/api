use crate::{
    error::ApiError,
    extract::{Body, Caller},
    state::AppState,
};
use application::ports::Connections;
use application::{AppError, Resource};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Html,
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
fn service(state: &AppState) -> Result<&Arc<dyn Connections>, ApiError> {
    state
        .ports
        .connections
        .as_ref()
        .ok_or_else(|| AppError::Unavailable("GitHub connections are disabled".into()).into())
}
#[derive(Serialize)]
struct Authorization {
    url: String,
}
async fn start(
    State(state): State<AppState>,
    Caller(actor): Caller,
) -> Result<Json<Authorization>, ApiError> {
    actor.require_user()?;
    let session = actor.session_id().ok_or(AppError::Unauthorized)?;
    Ok(Json(Authorization {
        url: service(&state)?.start(actor.user_id, session).await?,
    }))
}
#[derive(Deserialize)]
struct Callback {
    state: String,
    code: String,
}
async fn callback(
    State(state): State<AppState>,
    Query(query): Query<Callback>,
) -> Result<Html<&'static str>, ApiError> {
    service(&state)?.callback(&query.state, &query.code).await?;
    Ok(Html("<!doctype html><title>Account linked</title><p>Your GitHub account is linked. You can close this window and return to the app.</p>"))
}
async fn list(
    State(state): State<AppState>,
    Caller(actor): Caller,
) -> Result<Json<Vec<application::ports::Connection>>, ApiError> {
    actor.require_user()?;
    match state.ports.connections.as_ref() {
        Some(svc) => Ok(Json(svc.list(actor.user_id, false).await?)),
        None => Ok(Json(vec![])),
    }
}
async fn public_list(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
) -> Result<Json<Vec<application::ports::Connection>>, ApiError> {
    actor.require_user()?;
    let user = id.parse().map_err(|_| AppError::NotFound(Resource::User))?;
    match state.ports.connections.as_ref() {
        Some(svc) => Ok(Json(svc.list(user, true).await?)),
        None => Ok(Json(vec![])),
    }
}
#[derive(Deserialize)]
struct Visibility {
    visible: bool,
}
async fn update(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(provider): Path<String>,
    Body(body): Body<Visibility>,
) -> Result<StatusCode, ApiError> {
    actor.require_user()?;
    if provider != "github" {
        return Err(AppError::NotFound(Resource::User).into());
    }
    service(&state)?
        .visibility(actor.user_id, body.visible)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn delete(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(provider): Path<String>,
) -> Result<StatusCode, ApiError> {
    actor.require_user()?;
    if provider != "github" {
        return Err(AppError::NotFound(Resource::User).into());
    }
    service(&state)?.delete(actor.user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/users/@me/connections", get(list))
        .route(
            "/users/@me/connections/{provider}",
            put(update).delete(delete),
        )
        .route("/users/@me/connections/github/authorize", post(start))
        .route("/connections/github/callback", get(callback))
        .route("/users/{id}/connections", get(public_list))
}
