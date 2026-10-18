//! Outbox table access: enqueue (inside the unit of work), fetch and mark sent (relay).

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgConnection};

use super::error::db;
use crate::events::route::Routed;
use application::{AppError, AppResult};

/// A pending outbox row ready to publish.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingRecord {
    pub id: i64,
    pub topic: String,
    pub key: String,
    pub payload: serde_json::Value,
}

#[derive(FromRow)]
struct PendingRow {
    id: i64,
    topic: String,
    key: String,
    payload: serde_json::Value,
}

pub(super) async fn enqueue(
    conn: &mut PgConnection,
    routed: &Routed,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let payload = serde_json::to_value(&routed.envelope).map_err(AppError::internal)?;
    sqlx::query("INSERT INTO outbox (event_id, topic, key, payload, created_at) VALUES ($1, $2, $3, $4, $5)")
        .bind(routed.envelope.id)
        .bind(routed.topic)
        .bind(&routed.key)
        .bind(payload)
        .bind(now)
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}

/// Locks up to `limit` unsent rows for the caller's transaction; concurrent relays skip them.
pub async fn claim_unsent(conn: &mut PgConnection, limit: i64) -> AppResult<Vec<PendingRecord>> {
    let rows: Vec<PendingRow> = sqlx::query_as(
        "SELECT id, topic, key, payload FROM outbox WHERE sent_at IS NULL \
         ORDER BY id LIMIT $1 FOR UPDATE SKIP LOCKED",
    )
    .bind(limit)
    .fetch_all(conn)
    .await
    .map_err(db)?;
    Ok(rows
        .into_iter()
        .map(|row| PendingRecord {
            id: row.id,
            topic: row.topic,
            key: row.key,
            payload: row.payload,
        })
        .collect())
}

pub async fn mark_sent(conn: &mut PgConnection, ids: &[i64]) -> AppResult<()> {
    sqlx::query("UPDATE outbox SET sent_at = now() WHERE id = ANY($1)")
        .bind(ids)
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}
