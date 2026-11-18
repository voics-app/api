//! Passkey ceremonies use the configured RP, never request headers as an origin.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use std::time::Duration;
use uuid::Uuid;

use application::ports::PasskeyRecord;
use application::use_cases::{auth::webauthn, mfa::passkeys};
use contracts::passkeys as dto;

use super::auth::{client_info, to_response, AuthResponse};
use crate::error::ApiError;
use crate::extract::{Body, Caller};
use crate::ratelimit::strict;
use crate::state::AppState;

fn public_key(key: PasskeyRecord) -> dto::Passkey {
    dto::Passkey {
        id: key.id,
        name: key.name,
        created_at: key.created_at,
        last_used_at: key.last_used_at,
    }
}

fn challenge(value: passkeys::Challenge) -> Json<dto::Challenge> {
    Json(dto::Challenge {
        challenge_id: value.id,
        public_key: value.public_key,
    })
}

async fn list(
    State(state): State<AppState>,
    Caller(actor): Caller,
) -> Result<Json<Vec<dto::Passkey>>, ApiError> {
    Ok(Json(
        passkeys::list(&state.ports, &actor)
            .await?
            .into_iter()
            .map(public_key)
            .collect(),
    ))
}

async fn register_start(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Body(body): Body<dto::RegistrationStart>,
) -> Result<Json<dto::Challenge>, ApiError> {
    Ok(challenge(
        passkeys::start(&state.ports, &actor, &body.password, &body.name).await?,
    ))
}

async fn register_finish(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Body(body): Body<dto::RegistrationFinish>,
) -> Result<Json<dto::RegisteredPasskey>, ApiError> {
    let result = passkeys::finish(&state.ports, &actor, body.challenge_id, body.credential).await?;
    Ok(Json(dto::RegisteredPasskey {
        passkey: public_key(result.passkey),
        backup_codes: result.backup_codes.map(|codes| {
            codes
                .into_iter()
                .map(|code| dto::BackupCode {
                    code: code.code,
                    consumed: code.consumed,
                })
                .collect()
        }),
    }))
}

async fn remove(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<Uuid>,
    Body(body): Body<dto::RemovePasskey>,
) -> Result<StatusCode, ApiError> {
    passkeys::remove(&state.ports, &actor, id, &body.password, &body.code).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn auth_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Body(body): Body<dto::AuthenticationStart>,
) -> Result<Json<dto::Challenge>, ApiError> {
    strict(&state, &headers, "auth-mfa", 30, Duration::from_secs(600)).await?;
    Ok(challenge(
        webauthn::start(&state.ports, &body.ticket).await?,
    ))
}

async fn auth_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Body(body): Body<dto::AuthenticationFinish>,
) -> Result<Json<AuthResponse>, ApiError> {
    strict(&state, &headers, "auth-mfa", 30, Duration::from_secs(600)).await?;
    let tokens = webauthn::finish(
        &state.ports,
        &body.ticket,
        body.challenge_id,
        body.credential,
        client_info(&headers),
    )
    .await?;
    Ok(Json(to_response(&state, tokens).await?))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/users/@me/mfa/passkeys", get(list))
        .route(
            "/users/@me/mfa/passkeys/register/start",
            post(register_start),
        )
        .route(
            "/users/@me/mfa/passkeys/register/finish",
            post(register_finish),
        )
        .route("/users/@me/mfa/passkeys/{id}", delete(remove))
        .route("/auth/mfa/webauthn/start", post(auth_start))
        .route("/auth/mfa/webauthn/finish", post(auth_finish))
}
