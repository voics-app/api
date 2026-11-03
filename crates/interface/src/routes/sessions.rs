//! `/api/v1/users/@me/sessions` (P9): the caller's login sessions and remote logout.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use uuid::Uuid;

use application::use_cases::sessions::list::SessionView;
use application::use_cases::sessions::{list, logout_others, revoke};
use application::{AppError, Resource};
use contracts::objects::AuthSessionDto;
use contracts::wire::timestamp;

use crate::error::ApiError;
use crate::extract::Caller;
use crate::state::AppState;

fn dto(view: SessionView) -> AuthSessionDto {
    let session = view.active.session;
    AuthSessionDto {
        id: session.id.to_string(),
        client: session.client.client,
        os: session.client.os,
        device_name: view.active.device_name,
        location: session.client.location,
        created_at: timestamp(session.created_at),
        last_used_at: timestamp(session.last_used_at),
        current: view.current,
    }
}

async fn list_sessions(
    State(state): State<AppState>,
    Caller(actor): Caller,
) -> Result<Json<Vec<AuthSessionDto>>, ApiError> {
    let sessions = list::execute(&state.ports, &actor).await?;
    Ok(Json(sessions.into_iter().map(dto).collect()))
}

async fn revoke_session(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(session_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let session_id: Uuid = session_id
        .parse()
        .map_err(|_| AppError::NotFound(Resource::Session))?;
    revoke::execute(&state.ports, &actor, session_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn logout_other_sessions(
    State(state): State<AppState>,
    Caller(actor): Caller,
) -> Result<StatusCode, ApiError> {
    logout_others::execute(&state.ports, &actor).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/users/@me/sessions", get(list_sessions))
        .route(
            "/users/@me/sessions/logout-all",
            post(logout_other_sessions),
        )
        .route("/users/@me/sessions/{id}", delete(revoke_session))
}
