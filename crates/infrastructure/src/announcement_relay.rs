//! Durable announcement delivery. An ambiguous send is never repeated: an
//! operator must reconcile unknown outcomes against the reserved message id.
use std::sync::Arc;
use std::time::Duration;

use application::ports::MessageService;
use domain::Snowflake;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};

pub struct AnnouncementRelay {
    pool: PgPool,
    messages: Arc<dyn MessageService>,
}

impl AnnouncementRelay {
    pub fn new(pool: PgPool, messages: Arc<dyn MessageService>) -> Self {
        Self { pool, messages }
    }

    pub async fn run(self) {
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if let Err(error) = self.run_once().await {
                tracing::warn!(%error, "announcement relay failed");
            }
        }
    }

    pub async fn run_once(&self) -> Result<(), sqlx::Error> {
        // A crash after the claim may have completed the remote write. Never
        // resend it automatically, even if the destination was later deleted.
        sqlx::query("UPDATE announcement_deliveries SET state='unknown' WHERE state='claimed' AND claimed_at < now() - interval '5 minutes'")
            .execute(&self.pool).await?;
        // A response can be lost after messages stored the reserved identity.
        // Presence with the expected webhook proves success; absence is not
        // proof of failure because the delivered message may have been deleted.
        let unknown: Vec<(i64, i64, i64, i64, i64)> = sqlx::query_as("SELECT source_channel_id, source_message_id, target_channel_id, destination_message_id, webhook_id FROM announcement_deliveries WHERE state='unknown' ORDER BY claimed_at LIMIT 25")
            .fetch_all(&self.pool).await?;
        for (source, original, target, destination, webhook) in unknown {
            let found = self
                .messages
                .get(
                    Snowflake::new(target as u64),
                    Snowflake::new(destination as u64),
                )
                .await;
            if matches!(found, Ok(Some(ref message)) if message.body.get("webhook_id").and_then(Value::as_str) == Some(webhook.to_string().as_str()))
            {
                sqlx::query("UPDATE announcement_deliveries SET state='delivered' WHERE source_channel_id=$1 AND source_message_id=$2 AND target_channel_id=$3 AND state='unknown'")
                    .bind(source).bind(original).bind(target).execute(&self.pool).await?;
            }
        }
        let markers: Vec<(i64, i64)> = sqlx::query_as("SELECT source_channel_id, source_message_id FROM announcement_publications WHERE NOT marked ORDER BY created_at LIMIT 25")
            .fetch_all(&self.pool).await?;
        for (channel, message) in markers {
            if self
                .messages
                .mark_crossposted(
                    Snowflake::new(channel as u64),
                    Snowflake::new(message as u64),
                )
                .await
                .is_ok()
            {
                sqlx::query("UPDATE announcement_publications SET marked=true WHERE source_channel_id=$1 AND source_message_id=$2")
                    .bind(channel).bind(message).execute(&self.pool).await?;
            }
        }
        for _ in 0..10 {
            let mut tx = self.pool.begin().await?;
            let row = sqlx::query("SELECT d.source_channel_id, d.source_message_id, d.target_channel_id, d.destination_message_id, d.webhook_id, p.payload, w.name FROM announcement_deliveries d JOIN announcement_publications p USING (source_channel_id, source_message_id) JOIN announcement_followers f ON f.source_channel_id=d.source_channel_id AND f.target_channel_id=d.target_channel_id AND f.webhook_id=d.webhook_id JOIN webhooks w ON w.id=d.webhook_id WHERE d.state='pending' AND p.marked ORDER BY p.created_at, d.target_channel_id FOR UPDATE OF d SKIP LOCKED LIMIT 1")
                .fetch_optional(&mut *tx).await?;
            let Some(row) = row else {
                tx.rollback().await?;
                break;
            };
            let source: i64 = row.try_get("source_channel_id")?;
            let original: i64 = row.try_get("source_message_id")?;
            let target: i64 = row.try_get("target_channel_id")?;
            let destination: i64 = row.try_get("destination_message_id")?;
            let webhook: i64 = row.try_get("webhook_id")?;
            let snapshot: Value = row.try_get("payload")?;
            let name: String = row.try_get("name")?;
            sqlx::query("UPDATE announcement_deliveries SET state='claimed', claimed_at=now() WHERE source_channel_id=$1 AND source_message_id=$2 AND target_channel_id=$3 AND state='pending'")
                .bind(source).bind(original).bind(target).execute(&mut *tx).await?;
            tx.commit().await?;
            let body = json!({"source_channel_id": source.to_string(), "message_id": destination.to_string(), "webhook_id": webhook.to_string(), "webhook_name": name, "snapshot": snapshot});
            let state = if self
                .messages
                .deliver_crosspost(Snowflake::new(target as u64), body)
                .await
                .is_ok()
            {
                "delivered"
            } else {
                "unknown"
            };
            sqlx::query("UPDATE announcement_deliveries SET state=$4 WHERE source_channel_id=$1 AND source_message_id=$2 AND target_channel_id=$3 AND state='claimed'")
                .bind(source).bind(original).bind(target).bind(state).execute(&self.pool).await?;
        }
        Ok(())
    }
}
