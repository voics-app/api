//! `/api/v1/auth` (P9): register, login, refresh and logout.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde::Serialize;

use application::use_cases::auth::login::{self, LoginInput};
use application::use_cases::auth::logout;
use application::use_cases::auth::refresh;
use application::use_cases::auth::register::{self, RegisterInput};
use application::use_cases::auth::tokens::AuthTokens;
use application::AppResult;
use contracts::objects::UserDto;
use contracts::requests::{LoginRequest, RefreshRequest, RegisterRequest};

use crate::error::ApiError;
use crate::extract::{Body, Caller};
use crate::state::AppState;

#[derive(Serialize)]
pub struct AuthResponse {
    access_token: String,
    refresh_token: String,
    user: UserDto,
}

async fn to_response(state: &AppState, tokens: AuthTokens) -> AppResult<AuthResponse> {
    let locale = state.ports.users.settings(tokens.user.id).await?.locale;
    Ok(AuthResponse {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        user: UserDto::private(&tokens.user, &locale),
    })
}

async fn register(
    State(state): State<AppState>,
    Body(body): Body<RegisterRequest>,
) -> Result<(StatusCode, Json<AuthResponse>), ApiError> {
    let tokens = register::execute(
        &state.ports,
        RegisterInput {
            email: body.email,
            username: body.username,
            password: body.password,
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(to_response(&state, tokens).await?),
    ))
}

async fn login(
    State(state): State<AppState>,
    Body(body): Body<LoginRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    let tokens = login::execute(
        &state.ports,
        LoginInput {
            email: body.email,
            password: body.password,
            totp: body.totp,
            device_id: None,
        },
    )
    .await?;
    Ok(Json(to_response(&state, tokens).await?))
}

async fn refresh_tokens(
    State(state): State<AppState>,
    Body(body): Body<RefreshRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    let tokens = refresh::execute(&state.ports, &body.refresh_token).await?;
    Ok(Json(to_response(&state, tokens).await?))
}

async fn logout_session(
    State(state): State<AppState>,
    Caller(actor): Caller,
) -> Result<StatusCode, ApiError> {
    logout::execute(&state.ports, &actor).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/refresh", post(refresh_tokens))
        .route("/auth/logout", post(logout_session))
}
