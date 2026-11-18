use super::{convert::sf, error::db, PgStore};
use application::ports::{
    ChallengeKind, PasskeyChallenge, PasskeyRecord, PasskeyRepository, VerifiedCredential,
    MAX_PASSKEYS,
};
use application::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::Snowflake;
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

#[derive(FromRow)]
struct KeyRow {
    id: Uuid,
    user_id: i64,
    name: String,
    credential_id: Vec<u8>,
    credential: serde_json::Value,
    signature_counter: i64,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
}
impl From<KeyRow> for PasskeyRecord {
    fn from(row: KeyRow) -> Self {
        Self {
            id: row.id,
            user_id: sf(row.user_id),
            name: row.name,
            credential: VerifiedCredential {
                id: row.credential_id,
                material: row.credential,
                counter: row.signature_counter as u32,
            },
            created_at: row.created_at,
            last_used_at: row.last_used_at,
        }
    }
}

pub(super) async fn lock_account(conn: &mut PgConnection, user: Snowflake) -> AppResult<()> {
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(user.as_i64())
        .fetch_one(conn)
        .await
        .map_err(db)?;
    Ok(())
}

pub(super) async fn sync_mfa(conn: &mut PgConnection, user: Snowflake) -> AppResult<()> {
    sqlx::query("UPDATE users SET mfa_enabled = EXISTS(SELECT 1 FROM user_credentials WHERE user_id = $1 AND totp_secret IS NOT NULL) OR EXISTS(SELECT 1 FROM user_passkeys WHERE user_id = $1) WHERE id = $1")
        .bind(user.as_i64()).execute(&mut *conn).await.map_err(db)?;
    sqlx::query("DELETE FROM user_backup_codes WHERE user_id = $1 AND NOT EXISTS(SELECT 1 FROM users WHERE id = $1 AND mfa_enabled)")
        .bind(user.as_i64()).execute(conn).await.map_err(db)?;
    Ok(())
}

async fn notify_change(conn: &mut PgConnection, user: Snowflake) -> AppResult<()> {
    let row: super::users::UserRow = sqlx::query_as(&format!(
        "SELECT {} FROM users WHERE id = $1",
        super::users::USER_COLUMNS
    ))
    .bind(user.as_i64())
    .fetch_one(&mut *conn)
    .await
    .map_err(db)?;
    let now = Utc::now();
    // USER_UPDATE routing is independent of voice regions.
    for event in crate::events::route::route(
        &application::events::DomainEvent::UserUpdate(row.into()),
        "",
        now,
    ) {
        super::outbox::enqueue(&mut *conn, &event, now).await?;
    }
    Ok(())
}

#[async_trait]
impl PasskeyRepository for PgStore {
    async fn rename(
        &self,
        user: Snowflake,
        id: Uuid,
        name: &str,
    ) -> AppResult<Option<PasskeyRecord>> {
        let row: Option<KeyRow> = sqlx::query_as(
            "UPDATE user_passkeys SET name = $3 WHERE user_id = $1 AND id = $2 RETURNING *",
        )
        .bind(user.as_i64())
        .bind(id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row.map(Into::into))
    }

    async fn list(&self, user: Snowflake) -> AppResult<Vec<PasskeyRecord>> {
        let rows: Vec<KeyRow> = sqlx::query_as(
            "SELECT * FROM user_passkeys WHERE user_id = $1 ORDER BY created_at, id",
        )
        .bind(user.as_i64())
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn create_challenge(
        &self,
        challenge: &PasskeyChallenge,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        lock_account(&mut tx, challenge.user_id).await?;
        sqlx::query("DELETE FROM passkey_challenges WHERE id IN (SELECT id FROM passkey_challenges WHERE expires_at <= $1 LIMIT 1000)")
            .bind(now).execute(&mut *tx).await.map_err(db)?;
        // A bounded number of outstanding attempts per account, across all replicas.
        sqlx::query("DELETE FROM passkey_challenges WHERE id IN (SELECT id FROM passkey_challenges WHERE user_id = $1 ORDER BY created_at DESC, id DESC OFFSET 9)")
            .bind(challenge.user_id.as_i64()).execute(&mut *tx).await.map_err(db)?;
        sqlx::query("INSERT INTO passkey_challenges (id,user_id,kind,binding,name,state,expires_at,created_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(challenge.id).bind(challenge.user_id.as_i64()).bind(challenge.kind.as_str())
            .bind(&challenge.binding).bind(&challenge.name).bind(&challenge.state)
            .bind(challenge.expires_at).bind(now).execute(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)
    }

    async fn take_challenge(
        &self,
        id: Uuid,
        kind: ChallengeKind,
        binding: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<PasskeyChallenge>> {
        type Row = (i64, Option<String>, serde_json::Value, DateTime<Utc>);
        let row: Option<Row> = sqlx::query_as("DELETE FROM passkey_challenges WHERE id = $1 AND kind = $2 AND binding = $3 AND expires_at > $4 RETURNING user_id,name,state,expires_at")
            .bind(id).bind(kind.as_str()).bind(binding).bind(now).fetch_optional(&self.pool).await.map_err(db)?;
        Ok(row.map(|(user, name, state, expires_at)| PasskeyChallenge {
            id,
            user_id: sf(user),
            kind,
            binding: binding.to_owned(),
            name,
            state,
            expires_at,
        }))
    }

    async fn register(
        &self,
        record: &PasskeyRecord,
        recovery_hashes: &[String],
    ) -> AppResult<bool> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        lock_account(&mut tx, record.user_id).await?;
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM user_passkeys WHERE user_id = $1")
                .bind(record.user_id.as_i64())
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
        if count >= MAX_PASSKEYS as i64 {
            return Err(AppError::invalid(
                "credential",
                "PASSKEY_LIMIT",
                "At most ten passkeys may be registered.",
            ));
        }
        sqlx::query("INSERT INTO user_passkeys (id,user_id,name,credential_id,credential,signature_counter,created_at) VALUES ($1,$2,$3,$4,$5,$6,$7)")
            .bind(record.id).bind(record.user_id.as_i64()).bind(&record.name).bind(&record.credential.id)
            .bind(&record.credential.material).bind(i64::from(record.credential.counter)).bind(record.created_at)
            .execute(&mut *tx).await.map_err(db)?;
        let existing: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_backup_codes WHERE user_id = $1)")
                .bind(record.user_id.as_i64())
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
        if !existing {
            for hash in recovery_hashes {
                sqlx::query("INSERT INTO user_backup_codes (user_id,code_hash) VALUES ($1,$2)")
                    .bind(record.user_id.as_i64())
                    .bind(hash)
                    .execute(&mut *tx)
                    .await
                    .map_err(db)?;
            }
        }
        sync_mfa(&mut tx, record.user_id).await?;
        notify_change(&mut tx, record.user_id).await?;
        tx.commit().await.map_err(db)?;
        Ok(!existing)
    }

    async fn authenticate(
        &self,
        user: Snowflake,
        credential: &VerifiedCredential,
        ticket_hash: &str,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        // Same ticket has one winner even when different credentials are asserted concurrently.
        let consumed = sqlx::query("UPDATE account_tokens SET used_at = $3 WHERE user_id = $1 AND token_hash = $2 AND purpose = 'mfa_login' AND used_at IS NULL AND expires_at > $3")
            .bind(user.as_i64()).bind(ticket_hash).bind(now).execute(&mut *tx).await.map_err(db)?;
        if consumed.rows_affected() != 1 {
            return Ok(false);
        }
        let updated = sqlx::query("UPDATE user_passkeys SET credential = $3, signature_counter = $4, last_used_at = $5 WHERE user_id = $1 AND credential_id = $2 AND ((signature_counter = 0 AND $4 = 0) OR signature_counter < $4)")
            .bind(user.as_i64()).bind(&credential.id).bind(&credential.material).bind(i64::from(credential.counter))
            .bind(now).execute(&mut *tx).await.map_err(db)?;
        if updated.rows_affected() != 1 {
            return Ok(false);
        }
        tx.commit().await.map_err(db)?;
        Ok(true)
    }

    async fn remove(&self, user: Snowflake, id: Uuid) -> AppResult<bool> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        lock_account(&mut tx, user).await?;
        let removed = sqlx::query("DELETE FROM user_passkeys WHERE user_id = $1 AND id = $2")
            .bind(user.as_i64())
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sync_mfa(&mut tx, user).await?;
        if removed.rows_affected() == 1 {
            notify_change(&mut tx, user).await?;
        }
        tx.commit().await.map_err(db)?;
        Ok(removed.rows_affected() == 1)
    }
}
