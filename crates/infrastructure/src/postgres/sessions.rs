use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, Row};
use uuid::Uuid;

use application::ports::{ActiveSession, RefreshReplay, RotateOutcome, Session, SessionRepository};
use application::{AppError, AppResult};
use domain::session::ClientInfo;
use domain::Snowflake;

use super::convert::{sf, sf_opt};
use super::error::db;
use super::session_revocation;
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

#[derive(FromRow)]
struct RefreshTokenRow {
    session_id: Uuid,
    expires_at: DateTime<Utc>,
    rotated_at: Option<DateTime<Utc>>,
    rotation_request_hash: Option<String>,
    successor_hash: Option<String>,
}

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
        replay: Option<&RefreshReplay>,
    ) -> AppResult<RotateOutcome> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let token: Option<RefreshTokenRow> = sqlx::query_as(
            "SELECT session_id, expires_at, rotated_at, rotation_request_hash, successor_hash \
             FROM refresh_tokens WHERE token_hash = $1 FOR UPDATE",
        )
        .bind(old_hash)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        let Some(token) = token else {
            return Ok(RotateOutcome::Invalid);
        };
        let session_id = token.session_id;
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
        if let Some(rotated_at) = token.rotated_at {
            if replay
                .filter(|replay| {
                    rotated_at >= replay.earliest_rotation
                        && rotated_at <= now
                        && token.expires_at > now
                        && token.rotation_request_hash.as_deref()
                            == Some(replay.request_hash.as_str())
                        && token.successor_hash.as_deref() == Some(new_hash)
                })
                .is_some()
            {
                // The session lock serializes all rotations and revocations.
                // Do not lock the successor token here: another rotation may
                // already hold it while waiting for this same session lock.
                let usable: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM refresh_tokens WHERE token_hash = $1 \
                     AND session_id = $2 AND rotated_at IS NULL AND expires_at > $3)",
                )
                .bind(new_hash)
                .bind(session_id)
                .bind(now)
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
                if usable {
                    tx.commit().await.map_err(db)?;
                    return Ok(RotateOutcome::Rotated(session));
                }
            }
            sqlx::query("UPDATE sessions SET revoked_at = $2 WHERE id = $1")
                .bind(session_id)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            session_revocation::record(&mut tx, session.user_id, &[session_id], now).await?;
            tx.commit().await.map_err(db)?;
            session.revoked_at = Some(now);
            return Ok(RotateOutcome::ReuseDetected(session));
        }
        if token.expires_at <= now {
            return Ok(RotateOutcome::Invalid);
        }
        sqlx::query("UPDATE refresh_tokens SET rotated_at = $2, rotation_request_hash = $3, successor_hash = $4 WHERE token_hash = $1")
            .bind(old_hash)
            .bind(now)
            .bind(replay.map(|r| r.request_hash.as_str()))
            .bind(replay.map(|_| new_hash))
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
        let mut tx = self.pool.begin().await.map_err(db)?;
        let user: Option<i64> = sqlx::query_scalar(
            "UPDATE sessions SET revoked_at = $2 WHERE id = $1 AND revoked_at IS NULL RETURNING user_id",
        )
            .bind(id)
            .bind(now)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
        if let Some(user_id) = user {
            session_revocation::record(&mut tx, sf(user_id), &[id], now).await?;
        }
        tx.commit().await.map_err(db)
    }

    async fn revoke_all_for_user(
        &self,
        user_id: Snowflake,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<Uuid>> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let rows: Vec<(Uuid,)> = sqlx::query_as(
            "UPDATE sessions SET revoked_at = $2 WHERE user_id = $1 AND revoked_at IS NULL RETURNING id",
        )
        .bind(user_id.as_i64())
        .bind(now)
        .fetch_all(&mut *tx)
        .await
        .map_err(db)?;
        let ids: Vec<_> = rows.into_iter().map(|(id,)| id).collect();
        session_revocation::record(&mut tx, user_id, &ids, now).await?;
        tx.commit().await.map_err(db)?;
        Ok(ids)
    }

    async fn revoke_others(
        &self,
        user_id: Snowflake,
        keep: Uuid,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<Uuid>> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let rows: Vec<(Uuid,)> = sqlx::query_as(
            "UPDATE sessions SET revoked_at = $3              WHERE user_id = $1 AND id <> $2 AND revoked_at IS NULL RETURNING id",
        )
        .bind(user_id.as_i64())
        .bind(keep)
        .bind(now)
        .fetch_all(&mut *tx)
        .await
        .map_err(db)?;
        let ids: Vec<_> = rows.into_iter().map(|(id,)| id).collect();
        session_revocation::record(&mut tx, user_id, &ids, now).await?;
        tx.commit().await.map_err(db)?;
        Ok(ids)
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
        let mut tx = self.pool.begin().await.map_err(db)?;
        let user_id: Option<i64> = sqlx::query_scalar("SELECT user_id FROM sessions WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
        let user_id = user_id.ok_or(AppError::Unauthorized)?;
        super::push_tokens::lock_user(&mut tx, sf(user_id)).await?;
        let changed = sqlx::query(
            "UPDATE sessions SET device_id = $2 WHERE id = $1 AND revoked_at IS NULL \
             AND EXISTS (SELECT 1 FROM devices WHERE id = $2 AND user_id = $3)",
        )
        .bind(id)
        .bind(device_id.as_i64())
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        if changed.rows_affected() == 0 {
            return Err(AppError::Unauthorized);
        }
        // Push can be registered before E2EE startup finishes. Move its device
        // association with the session so old-device removal cannot remove a
        // token that already belongs to a replacement device.
        sqlx::query("UPDATE push_tokens SET device_id = $2 WHERE session_id = $1")
            .bind(id)
            .bind(device_id.as_i64())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)
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
