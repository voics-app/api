use async_trait::async_trait;
use chrono::{DateTime, Utc};

use application::ports::RelationshipRepository;
use application::AppResult;
use domain::relationship::{Relationship, RelationshipType};
use domain::Snowflake;

use super::convert::sf;
use super::error::db;
use super::PgStore;

type Row = (i64, i64, i16, DateTime<Utc>);

fn relationship((user_id, target_id, kind, since): Row) -> Option<Relationship> {
    Some(Relationship {
        user_id: sf(user_id),
        target_id: sf(target_id),
        kind: RelationshipType::from_code(kind as u8)?,
        since,
    })
}

#[async_trait]
impl RelationshipRepository for PgStore {
    async fn list(&self, user_id: Snowflake) -> AppResult<Vec<Relationship>> {
        let rows: Vec<Row> =
            sqlx::query_as("SELECT user_id, target_id, kind, since FROM relationships WHERE user_id = $1 ORDER BY since")
                .bind(user_id.as_i64())
                .fetch_all(&self.pool)
                .await
                .map_err(db)?;
        Ok(rows.into_iter().filter_map(relationship).collect())
    }

    async fn get(
        &self,
        user_id: Snowflake,
        target_id: Snowflake,
    ) -> AppResult<Option<Relationship>> {
        let row: Option<Row> = sqlx::query_as(
            "SELECT user_id, target_id, kind, since FROM relationships WHERE user_id = $1 AND target_id = $2",
        )
        .bind(user_id.as_i64())
        .bind(target_id.as_i64())
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row.and_then(relationship))
    }
}
