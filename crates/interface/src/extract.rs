//! Request extractors: the authenticated caller, JSON bodies that fail as
//! form errors, and the internal service token check.

use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::Json;
use serde::de::DeserializeOwned;
use subtle::ConstantTimeEq;

use application::use_cases::auth::authenticate::{self, Credential};
use application::{Actor, AppError};

use crate::error::ApiError;
use crate::state::AppState;

/// The caller, authenticated from `Authorization` (user JWT or bot token).
pub struct Caller(pub Actor);

impl FromRequestParts<AppState> for Caller {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or(AppError::Unauthorized)?;
        let actor = authenticate::execute(&state.ports, Credential::parse(header)).await?;
        Ok(Caller(actor))
    }
}

/// A JSON body; malformed bodies become a 400 form error instead of a plain rejection.
pub struct Body<T>(pub T);

impl<T> FromRequest<AppState> for Body<T>
where
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &AppState) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(Body(value)),
            Err(rejection) => {
                Err(AppError::invalid("body", "INVALID_BODY", rejection.body_text()).into())
            }
        }
    }
}

/// Requests from sibling services, authenticated by `X-Internal-Token` (P4).
pub struct Internal;

impl FromRequestParts<AppState> for Internal {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let presented = parts
            .headers
            .get("x-internal-token")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let expected = state.internal_token.as_bytes();
        if presented.as_bytes().ct_eq(expected).into() {
            Ok(Internal)
        } else {
            Err(AppError::Unauthorized.into())
        }
    }
}
