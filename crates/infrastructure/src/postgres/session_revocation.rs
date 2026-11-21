//! Revocation must reach connected gateways even if the HTTP request fails afterwards.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use application::AppResult;
use contracts::envelope::{Envelope, TOPIC_USER};
use contracts::Id;
use domain::Snowflake;

use super::{error::db, outbox};
use crate::events::route::Routed;

pub(super) async fn record(
    conn: &mut PgConnection,
    user_id: Snowflake,
    session_ids: &[Uuid],
    now: DateTime<Utc>,
) -> AppResult<()> {
    if session_ids.is_empty() {
        return Ok(());
    }
    for table in ["push_tokens", "voip_tokens"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE session_id = ANY($1)"))
            .bind(session_ids)
            .execute(&mut *conn)
            .await
            .map_err(db)?;
    }
    let event = Routed {
        topic: TOPIC_USER,
        key: user_id.to_string(),
        envelope: Envelope::new(
            Uuid::now_v7(),
            "SESSION_REVOKE",
            now,
            None,
            None,
            Some(vec![Id(user_id)]),
            serde_json::json!({"session_ids": session_ids}),
        ),
    };
    outbox::enqueue(conn, &event, now).await
}
