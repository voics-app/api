//! `/api/v1/users/@me/mfa` (P9): TOTP enrolment and backup codes.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde::Serialize;

use application::use_cases::mfa::backup_codes::{self, BackupCodeView};
use application::use_cases::mfa::{begin_enrolment, confirm_enrolment, disable};
use contracts::requests::{
    BackupCodesVerificationRequest, DisableTotpRequest, PasswordConfirmationRequest,
    VerifyTotpRequest,
};

use crate::error::ApiError;
use crate::extract::{Body, Caller};
use crate::state::AppState;

#[derive(Serialize)]
struct EnrolmentResponse {
    secret: String,
    otpauth_uri: String,
}

#[derive(Serialize)]
struct BackupCodeDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    consumed: bool,
}

#[derive(Serialize)]
struct BackupCodesResponse {
    backup_codes: Vec<BackupCodeDto>,
}

fn codes_response(views: Vec<BackupCodeView>) -> Json<BackupCodesResponse> {
    Json(BackupCodesResponse {
        backup_codes: views
            .into_iter()
            .map(|view| BackupCodeDto {
                code: view.code,
                consumed: view.consumed,
            })
            .collect(),
    })
}

async fn enable(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Body(body): Body<PasswordConfirmationRequest>,
) -> Result<Json<EnrolmentResponse>, ApiError> {
    let enrolment = begin_enrolment::execute(&state.ports, &actor, &body.password).await?;
    Ok(Json(EnrolmentResponse {
        secret: enrolment.secret,
        otpauth_uri: enrolment.otpauth_uri,
    }))
}

async fn verify(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Body(body): Body<VerifyTotpRequest>,
) -> Result<Json<BackupCodesResponse>, ApiError> {
    let codes = confirm_enrolment::execute(&state.ports, &actor, &body.code).await?;
    Ok(codes_response(
        codes
            .into_iter()
            .map(|code| BackupCodeView {
                code: Some(code),
                consumed: false,
            })
            .collect(),
    ))
}

async fn disable_totp(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Body(body): Body<DisableTotpRequest>,
) -> Result<StatusCode, ApiError> {
    disable::execute(&state.ports, &actor, &body.password, &body.code).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn codes_verification(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Body(body): Body<BackupCodesVerificationRequest>,
) -> Result<Json<BackupCodesResponse>, ApiError> {
    let views =
        backup_codes::execute(&state.ports, &actor, &body.password, body.regenerate).await?;
    Ok(codes_response(views))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/users/@me/mfa/totp/enable", post(enable))
        .route("/users/@me/mfa/totp/verify", post(verify))
        .route("/users/@me/mfa/totp/disable", post(disable_totp))
        .route(
            "/users/@me/mfa/codes-verification",
            post(codes_verification),
        )
}
