use domain::report::{Report, ReportStatus};
use domain::user::User;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;

pub const DEFAULT_LIMIT: u32 = 50;
pub const MAX_LIMIT: u32 = 100;

/// A report with the accounts it names, for the review queue.
#[derive(Debug, Clone)]
pub struct ReportView {
    pub report: Report,
    pub reporter: Option<User>,
    pub reported: Option<User>,
}

#[derive(Debug, Clone)]
pub struct ReportPage {
    pub reports: Vec<ReportView>,
    /// Cursor for the next page; `None` on the last page.
    pub next_after: Option<Snowflake>,
}

/// `GET /internal/v1/admin/reports?status=&limit=&after=`: newest first.
pub async fn execute(
    ports: &Ports,
    status: ReportStatus,
    limit: Option<u32>,
    after: Option<Snowflake>,
) -> AppResult<ReportPage> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let mut reports = ports.reports.list(status, limit + 1, after).await?;
    let next_after = if reports.len() > limit as usize {
        reports.truncate(limit as usize);
        reports.last().map(|r| r.id)
    } else {
        None
    };
    let mut ids: Vec<Snowflake> = reports
        .iter()
        .flat_map(|r| [r.reporter_id, r.reported_user_id])
        .collect();
    ids.sort();
    ids.dedup();
    let users = ports.users.find_many(&ids).await?;
    let find = |id: Snowflake| users.iter().find(|u| u.id == id).cloned();
    let reports = reports
        .into_iter()
        .map(|report| ReportView {
            reporter: find(report.reporter_id),
            reported: find(report.reported_user_id),
            report,
        })
        .collect();
    Ok(ReportPage {
        reports,
        next_after,
    })
}
