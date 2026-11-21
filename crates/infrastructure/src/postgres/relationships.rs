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
    async fn favourites(&self, user_id: Snowflake) -> AppResult<Vec<Snowflake>> {
        let rows: Vec<(i64,)> = sqlx::query_as(
            "SELECT target_id FROM friend_favourites WHERE user_id = $1 ORDER BY target_id",
        )
        .bind(user_id.as_i64())
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows.into_iter().map(|(id,)| sf(id)).collect())
    }

    async fn set_favourite(
        &self,
        user_id: Snowflake,
        target_id: Snowflake,
        favourite: bool,
    ) -> AppResult<bool> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let kind: Option<i16> = sqlx::query_scalar(
            "SELECT kind FROM relationships WHERE user_id = $1 AND target_id = $2 FOR UPDATE",
        )
        .bind(user_id.as_i64())
        .bind(target_id.as_i64())
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        if favourite && kind != Some(1) {
            return Ok(false);
        }
        if favourite {
            sqlx::query("INSERT INTO friend_favourites (user_id, target_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(user_id.as_i64()).bind(target_id.as_i64()).execute(&mut *tx).await.map_err(db)?;
        } else {
            sqlx::query("DELETE FROM friend_favourites WHERE user_id = $1 AND target_id = $2")
                .bind(user_id.as_i64())
                .bind(target_id.as_i64())
                .execute(&mut *tx)
                .await
                .map_err(db)?;
        }
        tx.commit().await.map_err(db)?;
        Ok(true)
    }

    async fn create_friend_invite(
        &self,
        owner: Snowflake,
        hash: &str,
        expires_at: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO friend_invites (owner_id, token_hash, expires_at) VALUES ($1, $2, $3) ON CONFLICT (owner_id) DO UPDATE SET token_hash = EXCLUDED.token_hash, expires_at = EXCLUDED.expires_at",
        )
        .bind(owner.as_i64())
        .bind(hash)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn friend_invite_owner(
        &self,
        hash: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<Snowflake>> {
        let id: Option<i64> = sqlx::query_scalar(
            "SELECT owner_id FROM friend_invites WHERE token_hash = $1 AND expires_at > $2",
        )
        .bind(hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(id.map(sf))
    }

    async fn revoke_friend_invites(&self, owner: Snowflake) -> AppResult<()> {
        sqlx::query("DELETE FROM friend_invites WHERE owner_id = $1")
            .bind(owner.as_i64())
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

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
