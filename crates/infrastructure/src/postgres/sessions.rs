use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, Row};
use uuid::Uuid;

use application::ports::{ActiveSession, RotateOutcome, Session, SessionRepository};
use application::AppResult;
use domain::session::ClientInfo;
use domain::Snowflake;

use super::convert::{sf, sf_opt};
use super::error::db;
use super::PgStore;

#[derive(FromRow)]
struct SessionRow {
    id: Uuid,
    user_id: i64,
    device_id: Option<i64>,
    client: String,
    os: String,
    location: Option<String>,
    created_at: DateTime<Utc>,
    last_used_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

impl From<SessionRow> for Session {
    fn from(row: SessionRow) -> Self {
        Session {
            id: row.id,
            user_id: sf(row.user_id),
            device_id: sf_opt(row.device_id),
            client: ClientInfo::restore(&row.client, &row.os, row.location.as_deref()),
            created_at: row.created_at,
            last_used_at: row.last_used_at,
            revoked_at: row.revoked_at,
        }
    }
}

const SESSION_COLUMNS: &str =
    "id, user_id, device_id, client, os, location, created_at, last_used_at, revoked_at";

#[async_trait]
impl SessionRepository for PgStore {
    async fn create(
        &self,
        session: &Session,
        refresh_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query(
            "INSERT INTO sessions (id, user_id, device_id, client, os, location, created_at, last_used_at)              VALUES ($1, $2, $3, $4, $5, $6, $7, $7)",
        )
        .bind(session.id)
        .bind(session.user_id.as_i64())
        .bind(session.device_id.map(|d| d.as_i64()))
        .bind(&session.client.client)
        .bind(&session.client.os)
        .bind(session.client.location.as_deref())
        .bind(session.created_at)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        sqlx::query(
            "INSERT INTO refresh_tokens (token_hash, session_id, expires_at) VALUES ($1, $2, $3)",
        )
        .bind(refresh_hash)
        .bind(session.id)
        .bind(expires_at)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(db)
    }

    async fn rotate(
        &self,
        old_hash: &str,
        new_hash: &str,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<RotateOutcome> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let token: Option<(Uuid, DateTime<Utc>, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT session_id, expires_at, rotated_at FROM refresh_tokens WHERE token_hash = $1 FOR UPDATE",
        )
        .bind(old_hash)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        let Some((session_id, token_expires_at, rotated_at)) = token else {
            return Ok(RotateOutcome::Invalid);
        };
        let sql = format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id = $1 FOR UPDATE");
        let row: SessionRow = sqlx::query_as(&sql)
            .bind(session_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
        let mut session = Session::from(row);
        if session.revoked_at.is_some() {
            return Ok(RotateOutcome::Invalid);
        }
        if rotated_at.is_some() {
            sqlx::query("UPDATE sessions SET revoked_at = $2 WHERE id = $1")
                .bind(session_id)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            tx.commit().await.map_err(db)?;
            session.revoked_at = Some(now);
            return Ok(RotateOutcome::ReuseDetected(session));
        }
        if token_expires_at <= now {
            return Ok(RotateOutcome::Invalid);
        }
        sqlx::query("UPDATE refresh_tokens SET rotated_at = $2 WHERE token_hash = $1")
            .bind(old_hash)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query(
            "INSERT INTO refresh_tokens (token_hash, session_id, expires_at) VALUES ($1, $2, $3)",
        )
        .bind(new_hash)
        .bind(session_id)
        .bind(expires_at)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        sqlx::query("UPDATE sessions SET last_used_at = $2 WHERE id = $1")
            .bind(session_id)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        session.last_used_at = now;
        Ok(RotateOutcome::Rotated(session))
    }

    async fn find(&self, id: Uuid) -> AppResult<Option<Session>> {
        let sql = format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id = $1");
        let row: Option<SessionRow> = sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        Ok(row.map(Session::from))
    }

    async fn revoke(&self, id: Uuid, now: DateTime<Utc>) -> AppResult<()> {
        sqlx::query("UPDATE sessions SET revoked_at = COALESCE(revoked_at, $2) WHERE id = $1")
            .bind(id)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    async fn revoke_all_for_user(
        &self,
        user_id: Snowflake,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<Uuid>> {
        let rows: Vec<(Uuid,)> = sqlx::query_as(
            "UPDATE sessions SET revoked_at = $2 WHERE user_id = $1 AND revoked_at IS NULL RETURNING id",
        )
        .bind(user_id.as_i64())
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    async fn revoke_others(
        &self,
        user_id: Snowflake,
        keep: Uuid,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<Uuid>> {
        let rows: Vec<(Uuid,)> = sqlx::query_as(
            "UPDATE sessions SET revoked_at = $3              WHERE user_id = $1 AND id <> $2 AND revoked_at IS NULL RETURNING id",
        )
        .bind(user_id.as_i64())
        .bind(keep)
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    async fn list_active(
        &self,
        user_id: Snowflake,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<ActiveSession>> {
        let rows: Vec<(SessionRow, Option<String>)> = sqlx::query(
            "SELECT s.id, s.user_id, s.device_id, s.client, s.os, s.location, s.created_at,                     s.last_used_at, s.revoked_at, d.name AS device_name              FROM sessions s LEFT JOIN devices d ON d.id = s.device_id              WHERE s.user_id = $1 AND s.revoked_at IS NULL                AND EXISTS (SELECT 1 FROM refresh_tokens r                            WHERE r.session_id = s.id AND r.rotated_at IS NULL AND r.expires_at > $2)              ORDER BY s.last_used_at DESC, s.id",
        )
        .bind(user_id.as_i64())
        .bind(now)
        .try_map(|row: sqlx::postgres::PgRow| {
            Ok((SessionRow::from_row(&row)?, row.try_get("device_name")?))
        })
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows
            .into_iter()
            .map(|(row, device_name)| ActiveSession {
                session: Session::from(row),
                device_name,
            })
            .collect())
    }

    async fn bind_device(&self, id: Uuid, device_id: Snowflake) -> AppResult<()> {
        sqlx::query("UPDATE sessions SET device_id = $2 WHERE id = $1")
            .bind(id)
            .bind(device_id.as_i64())
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    async fn live_device_ids(
        &self,
        user_id: Snowflake,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<Snowflake>> {
        let rows: Vec<(i64,)> = sqlx::query_as(
            "SELECT DISTINCT s.device_id FROM sessions s \
             WHERE s.user_id = $1 AND s.device_id IS NOT NULL AND s.revoked_at IS NULL \
               AND EXISTS (SELECT 1 FROM refresh_tokens r \
                           WHERE r.session_id = s.id AND r.rotated_at IS NULL AND r.expires_at > $2)",
        )
        .bind(user_id.as_i64())
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows.into_iter().map(|(id,)| sf(id)).collect())
    }
}
