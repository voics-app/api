//! `/api/v1/reports` (api#126): users report messages and users to the Trust and Safety queue.
//!
//! The route is not under `/channels/{id}/messages`: the ingress sends that whole prefix to the
//! messages service. Nothing here returns anything about the report, so a reporter cannot learn
//! whether someone else reported the same target, and no route exposes a report to the reported
//! user.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::Router;

use application::use_cases::reports::submit_report::{self, ReportTarget, SubmitReportInput};
use application::{AppError, Resource};
use contracts::requests::CreateReportRequest;
use domain::Snowflake;

use crate::error::ApiError;
use crate::extract::{Body, Caller};
use crate::state::AppState;

fn id(field: &str, raw: Option<&str>) -> Result<Snowflake, ApiError> {
    let missing = || AppError::invalid(field, "BASE_TYPE_REQUIRED", "This field is required.");
    raw.ok_or_else(missing)?.parse().map_err(|_| {
        AppError::invalid(field, "SNOWFLAKE_TYPE_INVALID", "Not a valid snowflake.").into()
    })
}

fn optional_id(field: &str, raw: Option<&str>) -> Result<Option<Snowflake>, ApiError> {
    raw.map(|value| id(field, Some(value))).transpose()
}

fn target(body: &CreateReportRequest) -> Result<ReportTarget, ApiError> {
    match body.target.as_str() {
        "message" => Ok(ReportTarget::Message {
            channel_id: id("channel_id", body.channel_id.as_deref())?,
            message_id: id("message_id", body.message_id.as_deref())?,
            author_id: optional_id("author_id", body.author_id.as_deref())?,
            content: body.content.clone(),
        }),
        "user" => Ok(ReportTarget::User {
            user_id: id("user_id", body.user_id.as_deref())
                .map_err(|_| AppError::NotFound(Resource::User))?,
        }),
        _ => Err(AppError::invalid(
            "target",
            "REPORT_TARGET_INVALID",
            "Must be message or user.",
        )
        .into()),
    }
}

async fn create_report(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Body(body): Body<CreateReportRequest>,
) -> Result<StatusCode, ApiError> {
    let input = SubmitReportInput {
        target: target(&body)?,
        category: body.category.clone(),
        note: body.note.clone(),
    };
    submit_report::execute(&state.ports, &actor, input).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/reports", post(create_report))
}
