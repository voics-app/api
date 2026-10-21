use async_trait::async_trait;
use chrono::{DateTime, Utc};

use application::ports::AccountTokenRepository;
use application::AppResult;
use domain::account::{AccountToken, TokenPurpose};
use domain::Snowflake;

use super::convert::sf;
use super::error::db;
use super::PgStore;

type TokenRow = (String, i64, String, Option<String>, DateTime<Utc>);

fn token(purpose: TokenPurpose, row: TokenRow) -> AccountToken {
    AccountToken {
        token_hash: row.0,
        user_id: sf(row.1),
        purpose,
        email: row.3,
        expires_at: row.4,
    }
}

#[async_trait]
impl AccountTokenRepository for PgStore {
    async fn create(&self, token: &AccountToken) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO account_tokens (token_hash, user_id, purpose, email, expires_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&token.token_hash)
        .bind(token.user_id.as_i64())
        .bind(token.purpose.as_str())
        .bind(&token.email)
        .bind(token.expires_at)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn find_active(
        &self,
        token_hash: &str,
        purpose: TokenPurpose,
        now: DateTime<Utc>,
    ) -> AppResult<Option<AccountToken>> {
        let row: Option<TokenRow> = sqlx::query_as(
            "SELECT token_hash, user_id, purpose, email, expires_at FROM account_tokens \
             WHERE token_hash = $1 AND purpose = $2 AND used_at IS NULL AND expires_at > $3",
        )
        .bind(token_hash)
        .bind(purpose.as_str())
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row.map(|row| token(purpose, row)))
    }

    async fn consume(
        &self,
        token_hash: &str,
        purpose: TokenPurpose,
        now: DateTime<Utc>,
    ) -> AppResult<Option<AccountToken>> {
        let row: Option<TokenRow> = sqlx::query_as(
            "UPDATE account_tokens SET used_at = $3 \
             WHERE token_hash = $1 AND purpose = $2 AND used_at IS NULL AND expires_at > $3 \
             RETURNING token_hash, user_id, purpose, email, expires_at",
        )
        .bind(token_hash)
        .bind(purpose.as_str())
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row.map(|row| token(purpose, row)))
    }

    async fn revoke_for_user(
        &self,
        user_id: Snowflake,
        purpose: TokenPurpose,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE account_tokens SET used_at = $3 \
             WHERE user_id = $1 AND purpose = $2 AND used_at IS NULL",
        )
        .bind(user_id.as_i64())
        .bind(purpose.as_str())
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }
}
