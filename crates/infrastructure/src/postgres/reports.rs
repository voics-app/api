use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::FromRow;

use application::ports::ReportRepository;
use application::{AppError, AppResult};
use domain::delivery_mode::DeliveryMode;
use domain::report::{ContentSource, Report, ReportCategory, ReportKind, ReportStatus};
use domain::Snowflake;

use super::convert::{mode, sf, sf_opt};
use super::error::db;
use super::PgStore;

const COLUMNS: &str = "id, reporter_id, reported_user_id, kind, category, note, channel_id, \
    guild_id, message_id, delivery_mode, content, content_source, status, created_at, \
    resolved_at, resolved_by, resolution_note";

#[derive(FromRow)]
struct ReportRow {
    id: i64,
    reporter_id: i64,
    reported_user_id: i64,
    kind: String,
    category: String,
    note: Option<String>,
    channel_id: Option<i64>,
    guild_id: Option<i64>,
    message_id: Option<i64>,
    delivery_mode: Option<String>,
    content: Option<String>,
    content_source: Option<String>,
    status: String,
    created_at: DateTime<Utc>,
    resolved_at: Option<DateTime<Utc>>,
    resolved_by: Option<String>,
    resolution_note: Option<String>,
}

impl TryFrom<ReportRow> for Report {
    type Error = AppError;

    fn try_from(row: ReportRow) -> Result<Self, AppError> {
        let stored = |what: &str| AppError::internal(format!("stored report {what} is unknown"));
        Ok(Report {
            id: sf(row.id),
            reporter_id: sf(row.reporter_id),
            reported_user_id: sf(row.reported_user_id),
            kind: ReportKind::parse(&row.kind).ok_or_else(|| stored("kind"))?,
            category: ReportCategory::parse(&row.category).map_err(|_| stored("category"))?,
            note: row.note,
            channel_id: sf_opt(row.channel_id),
            guild_id: sf_opt(row.guild_id),
            message_id: sf_opt(row.message_id),
            delivery_mode: row.delivery_mode.as_deref().map(mode),
            content: row.content,
            content_source: row
                .content_source
                .as_deref()
                .map(|raw| ContentSource::parse(raw).ok_or_else(|| stored("content source")))
                .transpose()?,
            status: ReportStatus::parse(&row.status).ok_or_else(|| stored("status"))?,
            created_at: row.created_at,
            resolved_at: row.resolved_at,
            resolved_by: row.resolved_by,
            resolution_note: row.resolution_note,
        })
    }
}

#[async_trait]
impl ReportRepository for PgStore {
    async fn insert(&self, report: &Report) -> AppResult<bool> {
        let inserted = sqlx::query(
            "INSERT INTO reports (id, reporter_id, reported_user_id, kind, category, note, \
             channel_id, guild_id, message_id, delivery_mode, content, content_source, status, \
             created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) \
             ON CONFLICT DO NOTHING",
        )
        .bind(report.id.as_i64())
        .bind(report.reporter_id.as_i64())
        .bind(report.reported_user_id.as_i64())
        .bind(report.kind.as_str())
        .bind(report.category.as_str())
        .bind(&report.note)
        .bind(report.channel_id.map(Snowflake::as_i64))
        .bind(report.guild_id.map(Snowflake::as_i64))
        .bind(report.message_id.map(Snowflake::as_i64))
        .bind(report.delivery_mode.map(DeliveryMode::as_str))
        .bind(&report.content)
        .bind(report.content_source.map(ContentSource::as_str))
        .bind(report.status.as_str())
        .bind(report.created_at)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(inserted.rows_affected() == 1)
    }

    async fn find(&self, id: Snowflake) -> AppResult<Option<Report>> {
        let row: Option<ReportRow> =
            sqlx::query_as(&format!("SELECT {COLUMNS} FROM reports WHERE id = $1"))
                .bind(id.as_i64())
                .fetch_optional(&self.pool)
                .await
                .map_err(db)?;
        row.map(Report::try_from).transpose()
    }

    async fn list(
        &self,
        status: ReportStatus,
        limit: u32,
        after: Option<Snowflake>,
    ) -> AppResult<Vec<Report>> {
        let rows: Vec<ReportRow> = sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM reports \
             WHERE status = $1 AND ($2::BIGINT IS NULL OR id < $2) \
             ORDER BY id DESC LIMIT $3"
        ))
        .bind(status.as_str())
        .bind(after.map(Snowflake::as_i64))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        rows.into_iter().map(Report::try_from).collect()
    }

    async fn close(
        &self,
        id: Snowflake,
        status: ReportStatus,
        by: &str,
        note: Option<&str>,
        at: DateTime<Utc>,
    ) -> AppResult<bool> {
        let closed = sqlx::query(
            "UPDATE reports SET status = $2, resolved_by = $3, resolution_note = $4, \
             resolved_at = $5 WHERE id = $1 AND status = 'open'",
        )
        .bind(id.as_i64())
        .bind(status.as_str())
        .bind(by)
        .bind(note)
        .bind(at)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(closed.rows_affected() == 1)
    }
}
