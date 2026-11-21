use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::Snowflake;

use crate::AppResult;

/// Atomic lifecycle transitions and their transactional gateway events.
#[async_trait]
pub trait ThreadLifecycleStore: Send + Sync {
    /// Archives up to `limit` due threads as of the supplied clock value.
    async fn archive_due(&self, now: DateTime<Utc>, limit: u32) -> AppResult<u64>;

    /// Records one committed message's timestamp. Duplicate and delayed deliveries are safe.
    async fn record_message(
        &self,
        thread_id: Snowflake,
        occurred_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<()>;
}
