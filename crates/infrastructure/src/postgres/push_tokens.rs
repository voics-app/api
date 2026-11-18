use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

use application::ports::PushTokenRepository;
use application::{AppError, AppResult};
use domain::push_token::{
    PushEnvironment, PushPlatform, PushToken, VoipToken, MAX_PUSH_TOKENS_PER_USER,
};
use domain::Snowflake;

use super::convert::{raw_ids, sf, sf_opt};
use super::error::db;
use super::PgStore;

/// Device binding/removal and token writes take this lock before any session
/// row lock. It closes the gap between finding bound sessions and deleting a
/// device, including tokens registered before the session acquired a device.
/// The namespace is separate from other advisory-lock uses; a hash collision
/// only serializes unrelated users and cannot grant access to their records.
pub(super) async fn lock_user(conn: &mut PgConnection, user_id: Snowflake) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('push-device:' || $1::text, 0))")
        .bind(user_id.as_i64())
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}

/// Recheck authorization inside the write transaction. Revocation updates the
/// same session row before deleting its tokens, so no late registration can
/// recreate a token after logout has committed.
async fn lock_live_session(
    conn: &mut PgConnection,
    session_id: Uuid,
    user_id: Snowflake,
) -> AppResult<Option<i64>> {
    lock_user(conn, user_id).await?;
    let row: Option<(Option<i64>,)> = sqlx::query_as(
        "SELECT device_id FROM sessions WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL FOR UPDATE",
    )
    .bind(session_id)
    .bind(user_id.as_i64())
    .fetch_optional(conn)
    .await
    .map_err(db)?;
    row.map(|(device_id,)| device_id)
        .ok_or(AppError::Unauthorized)
}

#[derive(FromRow)]
struct PushTokenRow {
    session_id: Uuid,
    user_id: i64,
    device_id: Option<i64>,
    platform: String,
    token: String,
    environment: String,
    previews: bool,
    only_mentions: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<PushTokenRow> for PushToken {
    type Error = AppError;

    fn try_from(row: PushTokenRow) -> Result<Self, AppError> {
        Ok(PushToken {
            session_id: row.session_id,
            user_id: sf(row.user_id),
            device_id: sf_opt(row.device_id),
            platform: PushPlatform::parse(&row.platform)
                .ok_or_else(|| AppError::internal("stored push platform is unknown"))?,
            token: row.token,
            environment: PushEnvironment::parse(&row.environment)
                .ok_or_else(|| AppError::internal("stored push environment is unknown"))?,
            previews: row.previews,
            only_mentions: row.only_mentions,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

#[derive(FromRow)]
struct VoipTokenRow {
    session_id: Uuid,
    user_id: i64,
    token: String,
    environment: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<VoipTokenRow> for VoipToken {
    type Error = AppError;

    fn try_from(row: VoipTokenRow) -> Result<Self, AppError> {
        Ok(VoipToken {
            session_id: row.session_id,
            user_id: sf(row.user_id),
            token: row.token,
            environment: PushEnvironment::parse(&row.environment)
                .ok_or_else(|| AppError::internal("stored push environment is unknown"))?,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

#[async_trait]
impl PushTokenRepository for PgStore {
    async fn upsert(&self, token: &PushToken) -> AppResult<()> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let device_id = lock_live_session(&mut tx, token.session_id, token.user_id).await?;
        // A provider token belongs to one installation: it moves to this session.
        sqlx::query(
            "DELETE FROM push_tokens WHERE platform = $1 AND token = $2 AND session_id <> $3",
        )
        .bind(token.platform.as_str())
        .bind(&token.token)
        .bind(token.session_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        sqlx::query(
            "INSERT INTO push_tokens \
                 (session_id, user_id, device_id, platform, token, environment, previews, only_mentions, \
                  created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9) \
             ON CONFLICT (session_id) DO UPDATE SET \
                 device_id = EXCLUDED.device_id, platform = EXCLUDED.platform, token = EXCLUDED.token, \
                 environment = EXCLUDED.environment, previews = EXCLUDED.previews, \
                 only_mentions = EXCLUDED.only_mentions, updated_at = EXCLUDED.updated_at",
        )
        .bind(token.session_id)
        .bind(token.user_id.as_i64())
        .bind(device_id)
        .bind(token.platform.as_str())
        .bind(&token.token)
        .bind(token.environment.as_str())
        .bind(token.previews)
        .bind(token.only_mentions)
        .bind(token.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        sqlx::query(
            "DELETE FROM push_tokens WHERE session_id IN ( \
                 SELECT session_id FROM push_tokens WHERE user_id = $1 \
                 ORDER BY updated_at DESC, session_id OFFSET $2)",
        )
        .bind(token.user_id.as_i64())
        .bind(MAX_PUSH_TOKENS_PER_USER as i64)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(db)
    }

    async fn delete_for_session(&self, session_id: Uuid) -> AppResult<bool> {
        let done = sqlx::query("DELETE FROM push_tokens WHERE session_id = $1")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        let voip = self.delete_voip_for_session(session_id).await?;
        Ok(done.rows_affected() > 0 || voip)
    }

    async fn delete_for_sessions(&self, session_ids: &[Uuid]) -> AppResult<()> {
        sqlx::query("DELETE FROM push_tokens WHERE session_id = ANY($1)")
            .bind(session_ids)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        sqlx::query("DELETE FROM voip_tokens WHERE session_id = ANY($1)")
            .bind(session_ids)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    async fn delete_by_token(&self, platform: PushPlatform, token: &str) -> AppResult<bool> {
        let done = sqlx::query("DELETE FROM push_tokens WHERE platform = $1 AND token = $2")
            .bind(platform.as_str())
            .bind(token)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        // The notifier reports a rejected token without saying which kind it was; VoIP tokens are APNs only.
        let voip = platform == PushPlatform::Apns
            && sqlx::query("DELETE FROM voip_tokens WHERE token = $1")
                .bind(token)
                .execute(&self.pool)
                .await
                .map_err(db)?
                .rows_affected()
                > 0;
        Ok(done.rows_affected() > 0 || voip)
    }

    async fn delete_stale(&self, cutoff: DateTime<Utc>) -> AppResult<u64> {
        let done = sqlx::query("DELETE FROM push_tokens WHERE updated_at < $1")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        let voip = sqlx::query("DELETE FROM voip_tokens WHERE updated_at < $1")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(done.rows_affected() + voip.rows_affected())
    }

    async fn list_live_for_users(&self, user_ids: &[Snowflake]) -> AppResult<Vec<PushToken>> {
        let rows: Vec<PushTokenRow> = sqlx::query_as(
            "SELECT p.session_id, p.user_id, p.device_id, p.platform, p.token, p.environment, \
                    p.previews, p.only_mentions, p.created_at, p.updated_at \
             FROM push_tokens p JOIN sessions s ON s.id = p.session_id \
             WHERE p.user_id = ANY($1) AND s.revoked_at IS NULL \
             ORDER BY p.user_id, p.session_id",
        )
        .bind(raw_ids(user_ids))
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        rows.into_iter().map(PushToken::try_from).collect()
    }

    async fn list_live_page(
        &self,
        after: Option<Snowflake>,
        limit: u32,
    ) -> AppResult<Vec<PushToken>> {
        let rows: Vec<PushTokenRow> = sqlx::query_as(
            "WITH page AS (                  SELECT p.user_id FROM push_tokens p JOIN sessions s ON s.id = p.session_id                  WHERE s.revoked_at IS NULL AND p.user_id > $1                  GROUP BY p.user_id ORDER BY p.user_id LIMIT $2)              SELECT p.session_id, p.user_id, p.device_id, p.platform, p.token, p.environment,                     p.previews, p.only_mentions, p.created_at, p.updated_at              FROM push_tokens p JOIN sessions s ON s.id = p.session_id              WHERE p.user_id IN (SELECT user_id FROM page) AND s.revoked_at IS NULL              ORDER BY p.user_id, p.session_id",
        )
        .bind(after.map_or(0, |id| id.as_i64()))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        rows.into_iter().map(PushToken::try_from).collect()
    }

    async fn upsert_voip(&self, token: &VoipToken) -> AppResult<()> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        lock_live_session(&mut tx, token.session_id, token.user_id).await?;
        // A PushKit token belongs to one installation: it moves to this session.
        sqlx::query("DELETE FROM voip_tokens WHERE token = $1 AND session_id <> $2")
            .bind(&token.token)
            .bind(token.session_id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query(
            "INSERT INTO voip_tokens (session_id, user_id, token, environment, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $5) \
             ON CONFLICT (session_id) DO UPDATE SET \
                 token = EXCLUDED.token, environment = EXCLUDED.environment, \
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(token.session_id)
        .bind(token.user_id.as_i64())
        .bind(&token.token)
        .bind(token.environment.as_str())
        .bind(token.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        sqlx::query(
            "DELETE FROM voip_tokens WHERE session_id IN ( \
                 SELECT session_id FROM voip_tokens WHERE user_id = $1 \
                 ORDER BY updated_at DESC, session_id OFFSET $2)",
        )
        .bind(token.user_id.as_i64())
        .bind(MAX_PUSH_TOKENS_PER_USER as i64)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(db)
    }

    async fn delete_voip_for_session(&self, session_id: Uuid) -> AppResult<bool> {
        let done = sqlx::query("DELETE FROM voip_tokens WHERE session_id = $1")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(done.rows_affected() > 0)
    }

    async fn list_live_voip_for_users(&self, user_ids: &[Snowflake]) -> AppResult<Vec<VoipToken>> {
        let rows: Vec<VoipTokenRow> = sqlx::query_as(
            "SELECT v.session_id, v.user_id, v.token, v.environment, v.created_at, v.updated_at \
             FROM voip_tokens v JOIN sessions s ON s.id = v.session_id \
             WHERE v.user_id = ANY($1) AND s.revoked_at IS NULL \
             ORDER BY v.user_id, v.session_id",
        )
        .bind(raw_ids(user_ids))
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        rows.into_iter().map(VoipToken::try_from).collect()
    }
}
