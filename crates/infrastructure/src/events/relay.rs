//! Outbox relay: publishes unsent rows to Redpanda and marks them sent.
//! Delivery is at-least-once; consumers deduplicate on the envelope `id`.

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;

use application::{AppError, AppResult};

use super::kafka::KafkaPublisher;
use crate::postgres::outbox;

const BATCH_SIZE: i64 = 200;
const IDLE_BACKOFF: Duration = Duration::from_millis(250);
const ERROR_BACKOFF: Duration = Duration::from_secs(2);

pub struct OutboxRelay {
    pool: PgPool,
    publisher: Arc<KafkaPublisher>,
}

impl OutboxRelay {
    pub fn new(pool: PgPool, publisher: Arc<KafkaPublisher>) -> Self {
        Self { pool, publisher }
    }

    /// Relays forever; transient failures back off and retry.
    pub async fn run(self) {
        loop {
            let pause = match self.relay_batch().await {
                Ok(0) => IDLE_BACKOFF,
                Ok(_) => Duration::ZERO,
                Err(error) => {
                    tracing::warn!(%error, "outbox relay batch failed");
                    ERROR_BACKOFF
                }
            };
            if !pause.is_zero() {
                tokio::time::sleep(pause).await;
            }
        }
    }

    /// Publishes one batch inside a transaction that holds its row locks.
    /// Returns the number of rows relayed.
    pub async fn relay_batch(&self) -> AppResult<usize> {
        let mut tx = self.pool.begin().await.map_err(AppError::internal)?;
        let records = outbox::claim_unsent(&mut tx, BATCH_SIZE).await?;
        if records.is_empty() {
            tx.commit().await.map_err(AppError::internal)?;
            return Ok(0);
        }
        let mut sent = Vec::with_capacity(records.len());
        for record in &records {
            let value = serde_json::to_vec(&record.payload).map_err(AppError::internal)?;
            self.publisher
                .publish(&record.topic, &record.key, value)
                .await?;
            sent.push(record.id);
        }
        outbox::mark_sent(&mut tx, &sent).await?;
        tx.commit().await.map_err(AppError::internal)?;
        Ok(sent.len())
    }
}
