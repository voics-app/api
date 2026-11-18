//! Liveness and readiness (every service exposes these, Global Constraints).

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;

use crate::state::AppState;

async fn liveness() -> &'static str {
    "ok"
}

/// Ready when the primary database answers and ID issuance remains valid.
async fn readiness(State(state): State<AppState>) -> StatusCode {
    if !state.ports.ids.is_ready() {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    match state.ports.guilds.count().await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(liveness))
        .route("/readyz", get(readiness))
}
