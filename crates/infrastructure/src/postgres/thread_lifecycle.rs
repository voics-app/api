//! Transactional thread lifecycle updates. The caller supplies the clock.

use super::{
    channels::{self, ChannelRow, CHANNEL_COLUMNS},
    convert::thread_metadata_json,
    error::db,
    outbox,
};
use application::{events::DomainEvent, ports::ThreadLifecycleStore, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::{channel::Channel, thread_lifecycle::ThreadLifecycle, Snowflake};
use sqlx::{PgConnection, PgPool};

pub struct PgThreadLifecycle {
    pool: PgPool,
    region: String,
}
impl PgThreadLifecycle {
    pub fn new(pool: PgPool, region: String) -> Self {
        Self { pool, region }
    }

    async fn apply(
        &self,
        conn: &mut PgConnection,
        mut channel: Channel,
        now: DateTime<Utc>,
        message: Option<DateTime<Utc>>,
    ) -> AppResult<bool> {
        let (last_activity_at, automatically_archived): (DateTime<Utc>, bool) = sqlx::query_as(
            "SELECT last_activity_at, automatically_archived FROM thread_activity WHERE thread_id = $1 FOR UPDATE"
        ).bind(channel.id.as_i64()).fetch_one(&mut *conn).await.map_err(db)?;
        let Some(meta) = channel.thread_metadata.as_mut() else {
            return Ok(false);
        };
        let mut lifecycle = ThreadLifecycle {
            archived: meta.archived,
            locked: meta.locked,
            duration_minutes: meta.auto_archive_duration,
            last_activity_at,
            archive_timestamp: meta.archive_timestamp,
            automatically_archived,
        };
        let changed = match message {
            Some(at) => lifecycle.record_message(at.min(now), now),
            None => lifecycle.archive_if_due(now),
        };
        if changed {
            meta.archived = lifecycle.archived;
            meta.archive_timestamp = lifecycle.archive_timestamp;
            sqlx::query("UPDATE channels SET thread_metadata = $2 WHERE id = $1")
                .bind(channel.id.as_i64())
                .bind(thread_metadata_json(meta))
                .execute(&mut *conn)
                .await
                .map_err(db)?;
        }
        sqlx::query("UPDATE thread_activity SET last_activity_at = $2, automatically_archived = $3 WHERE thread_id = $1")
            .bind(channel.id.as_i64()).bind(lifecycle.last_activity_at).bind(lifecycle.automatically_archived)
            .execute(&mut *conn).await.map_err(db)?;
        if changed {
            for event in
                crate::events::route::route(&DomainEvent::ThreadUpdate(channel), &self.region, now)
            {
                outbox::enqueue(&mut *conn, &event, now).await?;
            }
        }
        Ok(changed)
    }
}

#[async_trait]
impl ThreadLifecycleStore for PgThreadLifecycle {
    async fn archive_due(&self, now: DateTime<Utc>, limit: u32) -> AppResult<u64> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let sql = format!("SELECT c.{} FROM channels c JOIN thread_activity a ON a.thread_id = c.id \
            WHERE c.kind IN (10,11,12) AND c.thread_metadata->>'archived' = 'false' \
            AND c.thread_metadata->>'auto_archive_duration' IN ('60','1440','4320','10080') \
            AND a.last_activity_at + ((c.thread_metadata->>'auto_archive_duration')::int * INTERVAL '1 minute') <= $1 \
            ORDER BY a.last_activity_at, c.id LIMIT $2 FOR UPDATE OF c SKIP LOCKED", CHANNEL_COLUMNS.replace(", ", ", c."));
        let rows: Vec<ChannelRow> = sqlx::query_as(&sql)
            .bind(now)
            .bind(i64::from(limit))
            .fetch_all(&mut *tx)
            .await
            .map_err(db)?;
        let channels = channels::hydrate_on(&mut tx, rows).await.map_err(db)?;
        let mut count = 0;
        for channel in channels {
            if self.apply(&mut tx, channel, now, None).await? {
                count += 1;
            }
        }
        tx.commit().await.map_err(db)?;
        Ok(count)
    }

    async fn record_message(
        &self,
        thread_id: Snowflake,
        occurred_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let sql = format!("SELECT {CHANNEL_COLUMNS} FROM channels WHERE id = $1 AND kind IN (10,11,12) FOR UPDATE");
        let row: Option<ChannelRow> = sqlx::query_as(&sql)
            .bind(thread_id.as_i64())
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
        if let Some(row) = row {
            if let Some(channel) = channels::hydrate_on(&mut tx, vec![row])
                .await
                .map_err(db)?
                .pop()
            {
                self.apply(&mut tx, channel, now, Some(occurred_at)).await?;
            }
        }
        tx.commit().await.map_err(db)?;
        Ok(())
    }
}
