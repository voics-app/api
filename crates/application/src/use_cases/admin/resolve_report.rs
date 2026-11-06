use domain::report::{Report, Resolution};
use domain::Snowflake;

use crate::context::Ports;
use crate::error::{AppError, AppResult, Resource};

const MAX_NOTE_CHARS: usize = 500;
const MAX_REVIEWER_CHARS: usize = 320;

/// `POST /internal/v1/admin/reports/{id}/resolve`: closes an open report as resolved or
/// dismissed. `reviewer` is the operator the admin portal acts for.
pub async fn execute(
    ports: &Ports,
    id: Snowflake,
    resolution: Resolution,
    reviewer: &str,
    note: Option<&str>,
) -> AppResult<Report> {
    let reviewer = reviewer.trim();
    if reviewer.is_empty() || reviewer.chars().count() > MAX_REVIEWER_CHARS {
        return Err(AppError::invalid(
            "resolved_by",
            "BASE_TYPE_BAD_LENGTH",
            "Must be between 1 and 320 in length.",
        ));
    }
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    if note.is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS) {
        return Err(AppError::invalid(
            "note",
            "BASE_TYPE_BAD_LENGTH",
            "Must be 500 or fewer in length.",
        ));
    }
    ports
        .reports
        .find(id)
        .await?
        .ok_or(AppError::NotFound(Resource::Report))?;
    let closed = ports
        .reports
        .close(id, resolution.status(), reviewer, note, ports.clock.now())
        .await?;
    if !closed {
        return Err(AppError::Conflict {
            code: 0,
            message: "The report is already closed.".into(),
        });
    }
    ports
        .reports
        .find(id)
        .await?
        .ok_or(AppError::NotFound(Resource::Report))
}
