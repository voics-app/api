//! Maps application errors onto Discord-style HTTP responses (P9 errors).

use std::collections::BTreeMap;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::{json, Value};

use application::{AppError, Denial, Resource};
use contracts::errors::{
    BANNED, CANNOT_MESSAGE_USER, GENERAL_ERROR, INVALID_FORM_BODY, MFA_REQUIRED, MISSING_ACCESS,
    MISSING_PERMISSIONS, RATE_LIMITED, UNAUTHORIZED, UNKNOWN_APPLICATION,
    UNKNOWN_APPLICATION_COMMAND, UNKNOWN_BAN, UNKNOWN_CHANNEL, UNKNOWN_EMOJI, UNKNOWN_GUILD,
    UNKNOWN_GUILD_TEMPLATE, UNKNOWN_INTERACTION, UNKNOWN_INVITE, UNKNOWN_MEMBER, UNKNOWN_MESSAGE,
    UNKNOWN_ROLE, UNKNOWN_USER, UNKNOWN_WEBHOOK,
};

/// An error ready to be rendered as `{ code, message, errors? }`.
#[derive(Debug)]
pub struct ApiError(pub AppError);

impl From<AppError> for ApiError {
    fn from(error: AppError) -> Self {
        ApiError(error)
    }
}

#[derive(Serialize)]
struct Body {
    code: u32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    errors: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ticket: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    methods: Option<Vec<&'static str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after: Option<u64>,
}

fn unknown_code(resource: Resource) -> u32 {
    match resource {
        Resource::Application => UNKNOWN_APPLICATION,
        Resource::Ban => UNKNOWN_BAN,
        Resource::Channel | Resource::Overwrite => UNKNOWN_CHANNEL,
        Resource::Command => UNKNOWN_APPLICATION_COMMAND,
        Resource::Emoji => UNKNOWN_EMOJI,
        Resource::SoundboardSound => GENERAL_ERROR,
        Resource::Guild => UNKNOWN_GUILD,
        Resource::GuildTemplate => UNKNOWN_GUILD_TEMPLATE,
        Resource::Interaction => UNKNOWN_INTERACTION,
        Resource::Invite => UNKNOWN_INVITE,
        Resource::Member => UNKNOWN_MEMBER,
        Resource::Message => UNKNOWN_MESSAGE,
        Resource::Role => UNKNOWN_ROLE,
        Resource::User => UNKNOWN_USER,
        Resource::Webhook => UNKNOWN_WEBHOOK,
        Resource::Device | Resource::KeyPackage | Resource::Report | Resource::Session => {
            GENERAL_ERROR
        }
    }
}

fn denial_code(denial: Denial) -> (StatusCode, u32, &'static str) {
    match denial {
        Denial::MissingAccess => (StatusCode::FORBIDDEN, MISSING_ACCESS, "Missing access"),
        Denial::MissingPermissions => (
            StatusCode::FORBIDDEN,
            MISSING_PERMISSIONS,
            "Missing permissions",
        ),
        Denial::BotsNotAllowed => (
            StatusCode::FORBIDDEN,
            MISSING_ACCESS,
            "Bots cannot use this endpoint",
        ),
        Denial::Banned => (
            StatusCode::FORBIDDEN,
            BANNED,
            "You are banned from this guild",
        ),
        Denial::CannotMessageUser => (
            StatusCode::FORBIDDEN,
            CANNOT_MESSAGE_USER,
            "Cannot send messages to this user",
        ),
        Denial::Suspended => (
            StatusCode::FORBIDDEN,
            GENERAL_ERROR,
            "This account is suspended",
        ),
    }
}

/// `{ "field": { "_errors": [{ "code", "message" }] } }`, Discord's field error shape.
fn field_errors(errors: &domain::validation::ValidationErrors) -> Value {
    let mut grouped: BTreeMap<&str, Vec<Value>> = BTreeMap::new();
    for error in errors.errors() {
        grouped
            .entry(error.field.as_str())
            .or_default()
            .push(json!({ "code": error.code, "message": error.message }));
    }
    let object: serde_json::Map<String, Value> = grouped
        .into_iter()
        .map(|(field, items)| (field.to_owned(), json!({ "_errors": items })))
        .collect();
    Value::Object(object)
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut ticket = None;
        let mut methods = None;
        let mut retry_after = None;
        let (status, code, message, errors) = match self.0 {
            AppError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                UNAUTHORIZED,
                "401: Unauthorized".to_owned(),
                None,
            ),
            AppError::MfaRequired {
                ticket: issued,
                methods: available,
            } => {
                ticket = Some(issued);
                methods = Some(available);
                (
                    StatusCode::UNAUTHORIZED,
                    MFA_REQUIRED,
                    "Two-factor authentication required".to_owned(),
                    None,
                )
            }
            AppError::RateLimited { retry_after_secs } => {
                retry_after = Some(retry_after_secs);
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    RATE_LIMITED,
                    "You are being rate limited.".to_owned(),
                    None,
                )
            }
            AppError::Forbidden(denial) => {
                let (status, code, message) = denial_code(denial);
                (status, code, message.to_owned(), None)
            }
            AppError::NotFound(resource) => (
                StatusCode::NOT_FOUND,
                unknown_code(resource),
                format!("Unknown {}", format!("{resource:?}").to_lowercase()),
                None,
            ),
            AppError::Validation(errors) => (
                StatusCode::BAD_REQUEST,
                INVALID_FORM_BODY,
                "Invalid form body".to_owned(),
                Some(field_errors(&errors)),
            ),
            AppError::Conflict { code, message } => (StatusCode::CONFLICT, code, message, None),
            AppError::BadRequest { code, message } => {
                (StatusCode::BAD_REQUEST, code, message, None)
            }
            AppError::Unavailable(detail) => {
                tracing::warn!(%detail, "upstream unavailable");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    GENERAL_ERROR,
                    "Service unavailable".to_owned(),
                    None,
                )
            }
            AppError::Internal(detail) => {
                tracing::error!(%detail, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    GENERAL_ERROR,
                    "Internal server error".to_owned(),
                    None,
                )
            }
        };
        let mut response = (
            status,
            Json(Body {
                code,
                message,
                errors,
                ticket,
                methods,
                retry_after,
            }),
        )
            .into_response();
        if let Some(seconds) = retry_after {
            response.headers_mut().insert(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from(seconds),
            );
        }
        response
    }
}
