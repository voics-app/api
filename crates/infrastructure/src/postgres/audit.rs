//! Audit log reads.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::FromRow;

use application::ports::{AuditLogQuery, AuditLogRepository};
use application::AppResult;
use domain::audit::{AuditAction, AuditChange, AuditLogEntry};
use domain::Snowflake;

use super::convert::{sf, sf_opt};
use super::error::db;
use super::PgStore;

#[derive(FromRow)]
struct AuditRow {
    id: i64,
    guild_id: i64,
    user_id: Option<i64>,
    target_id: Option<i64>,
    action: i16,
    changes: serde_json::Value,
    reason: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct StoredChange {
    key: String,
    old_value: Option<serde_json::Value>,
    new_value: Option<serde_json::Value>,
}

impl TryFrom<AuditRow> for AuditLogEntry {
    type Error = application::AppError;

    fn try_from(row: AuditRow) -> Result<Self, Self::Error> {
        let action = AuditAction::from_code(row.action as u16).ok_or_else(|| {
            application::AppError::internal(format!("unknown audit action {}", row.action))
        })?;
        let changes: Vec<StoredChange> =
            serde_json::from_value(row.changes).map_err(application::AppError::internal)?;
        Ok(AuditLogEntry {
            id: sf(row.id),
            guild_id: sf(row.guild_id),
            user_id: sf_opt(row.user_id),
            target_id: sf_opt(row.target_id),
            action,
            changes: changes
                .into_iter()
                .map(|c| AuditChange {
                    key: c.key,
                    old_value: c.old_value,
                    new_value: c.new_value,
                })
                .collect(),
            reason: row.reason,
            created_at: row.created_at,
        })
    }
}

#[async_trait]
impl AuditLogRepository for PgStore {
    async fn list(
        &self,
        guild_id: Snowflake,
        query: AuditLogQuery,
    ) -> AppResult<Vec<AuditLogEntry>> {
        let rows: Vec<AuditRow> = sqlx::query_as(
            "SELECT id, guild_id, user_id, target_id, action, changes, reason, created_at FROM audit_log \
             WHERE guild_id = $1 \
               AND ($2::bigint IS NULL OR user_id = $2) \
               AND ($3::smallint IS NULL OR action = $3) \
               AND ($4::bigint IS NULL OR id < $4) \
             ORDER BY id DESC LIMIT $5",
        )
        .bind(guild_id.as_i64())
        .bind(query.user_id.map(|id| id.as_i64()))
        .bind(query.action.map(|a| a.code() as i16))
        .bind(query.before.map(|id| id.as_i64()))
        .bind(i64::from(query.limit))
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        rows.into_iter().map(AuditLogEntry::try_from).collect()
    }
}
