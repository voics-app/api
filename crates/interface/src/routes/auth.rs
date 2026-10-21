//! `/api/v1/auth` (P9): register, login, refresh, logout, login MFA, email
//! verification and password recovery.

use std::time::Duration;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde::Serialize;

use application::use_cases::auth::login::{self, LoginInput};
use application::use_cases::auth::login_mfa::{self, MfaLoginInput};
use application::use_cases::auth::logout;
use application::use_cases::auth::refresh;
use application::use_cases::auth::register::{self, RegisterInput};
use application::use_cases::auth::tokens::AuthTokens;
use application::use_cases::auth::{
    forgot_password, resend_verification, reset_password, verify_email,
};
use application::AppResult;
use contracts::objects::UserDto;
use contracts::requests::{
    ForgotPasswordRequest, LoginRequest, MfaLoginRequest, RefreshRequest, RegisterRequest,
    ResetPasswordRequest, VerifyEmailRequest,
};

use crate::error::ApiError;
use crate::extract::{Body, Caller};
use crate::ratelimit::strict;
use crate::state::AppState;

const HOUR: Duration = Duration::from_secs(3600);
const TEN_MINUTES: Duration = Duration::from_secs(600);

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

async fn login_totp(
    State(state): State<AppState>,
    headers: HeaderMap,
    Body(body): Body<MfaLoginRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    strict(&state, &headers, "auth-mfa", 30, TEN_MINUTES).await?;
    let tokens = login_mfa::execute(
        &state.ports,
        MfaLoginInput {
            ticket: body.ticket,
            code: body.code,
            device_id: None,
        },
    )
    .await?;
    Ok(Json(to_response(&state, tokens).await?))
}

async fn forgot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Body(body): Body<ForgotPasswordRequest>,
) -> Result<StatusCode, ApiError> {
    strict(&state, &headers, "auth-forgot", 5, HOUR).await?;
    forgot_password::execute(&state.ports, &body.email).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reset(
    State(state): State<AppState>,
    headers: HeaderMap,
    Body(body): Body<ResetPasswordRequest>,
) -> Result<StatusCode, ApiError> {
    strict(&state, &headers, "auth-reset", 10, HOUR).await?;
    reset_password::execute(&state.ports, &body.token, &body.password).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Body(body): Body<VerifyEmailRequest>,
) -> Result<StatusCode, ApiError> {
    strict(&state, &headers, "auth-verify", 20, HOUR).await?;
    verify_email::execute(&state.ports, &body.token).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn resend(
    State(state): State<AppState>,
    Caller(actor): Caller,
) -> Result<StatusCode, ApiError> {
    resend_verification::execute(&state.ports, &actor).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/refresh", post(refresh_tokens))
        .route("/auth/logout", post(logout_session))
        .route("/auth/mfa/totp", post(login_totp))
        .route("/auth/forgot-password", post(forgot))
        .route("/auth/reset-password", post(reset))
        .route("/auth/verify-email", post(verify))
        .route("/auth/resend-verification", post(resend))
}
