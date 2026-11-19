//! GitHub OAuth consumer. Provider tokens are used once and never stored.
use crate::security::secrets::OsSecrets;
use application::ports::{Connection, Connections, SecretGenerator};
use application::{AppError, AppResult};
use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use domain::Snowflake;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::time::Duration;
use url::Url;
use uuid::Uuid;

pub struct GithubConnections {
    pool: PgPool,
    client: reqwest::Client,
    client_id: String,
    client_secret: String,
    callback: Url,
    authorize_url: Url,
    token_url: Url,
    user_url: Url,
}
impl GithubConnections {
    pub fn new(
        pool: PgPool,
        client_id: String,
        client_secret: String,
        callback: Url,
    ) -> AppResult<Self> {
        Self::with_endpoints(
            pool,
            client_id,
            client_secret,
            callback,
            Url::parse("https://github.com/login/oauth/authorize").unwrap(),
            Url::parse("https://github.com/login/oauth/access_token").unwrap(),
            Url::parse("https://api.github.com/user").unwrap(),
        )
    }
    /// Alternate endpoints are intended solely for controlled integration tests.
    pub fn with_endpoints(
        pool: PgPool,
        client_id: String,
        client_secret: String,
        callback: Url,
        authorize_url: Url,
        token_url: Url,
        user_url: Url,
    ) -> AppResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("voics-connections")
            .build()
            .map_err(AppError::internal)?;
        Ok(Self {
            pool,
            client,
            client_id,
            client_secret,
            callback,
            authorize_url,
            token_url,
            user_url,
        })
    }
}
#[derive(Deserialize)]
struct Token {
    access_token: String,
}
#[derive(Deserialize)]
struct GithubUser {
    id: u64,
    login: String,
}
fn upstream() -> AppError {
    AppError::Unavailable("GitHub account verification failed".into())
}
fn invalid_state() -> AppError {
    AppError::invalid(
        "state",
        "INVALID_STATE",
        "OAuth session expired or already used",
    )
}
#[async_trait]
impl Connections for GithubConnections {
    async fn start(&self, user: Snowflake, session: Uuid) -> AppResult<String> {
        if self.client_id.is_empty() || self.client_secret.is_empty() {
            return Err(AppError::Unavailable(
                "GitHub connections are disabled".into(),
            ));
        }
        sqlx::query("DELETE FROM connection_oauth_states WHERE expires_at <= now()")
            .execute(&self.pool)
            .await
            .map_err(AppError::internal)?;
        let state = OsSecrets.token(32);
        let verifier = OsSecrets.token(32);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        sqlx::query("INSERT INTO connection_oauth_states (state_hash,user_id,session_id,verifier,expires_at) VALUES ($1,$2,$3,$4,now()+interval '10 minutes')")
            .bind(OsSecrets.hash(&state)).bind(user.as_i64()).bind(session)
            .bind(&verifier)
            .execute(&self.pool).await.map_err(AppError::internal)?;
        let mut url = self.authorize_url.clone();
        url.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", self.callback.as_str())
            .append_pair("state", &state)
            .append_pair("scope", "read:user")
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
        Ok(url.into())
    }
    async fn callback(&self, state: &str, code: &str) -> AppResult<()> {
        if state.len() > 128 || code.len() > 2048 || state.is_empty() || code.is_empty() {
            return Err(invalid_state());
        }
        // DELETE consumes the state atomically before exchanging a code. Revoked sessions cannot link.
        let user: Option<(i64, String)> = sqlx::query_as("DELETE FROM connection_oauth_states s USING sessions x WHERE s.state_hash=$1 AND s.expires_at>now() AND x.id=s.session_id AND x.user_id=s.user_id AND x.revoked_at IS NULL RETURNING s.user_id,s.verifier")
            .bind(OsSecrets.hash(state)).fetch_optional(&self.pool).await.map_err(AppError::internal)?;
        let (user, verifier) = user.ok_or_else(invalid_state)?;
        let token: Token = self
            .client
            .post(self.token_url.clone())
            .header("Accept", "application/json")
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("code", code),
                ("redirect_uri", self.callback.as_str()),
                ("state", state),
                ("code_verifier", verifier.as_str()),
            ])
            .send()
            .await
            .map_err(|_| upstream())?
            .error_for_status()
            .map_err(|_| upstream())?
            .json()
            .await
            .map_err(|_| upstream())?;
        let profile: GithubUser = self
            .client
            .get(self.user_url.clone())
            .bearer_auth(token.access_token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|_| upstream())?
            .error_for_status()
            .map_err(|_| upstream())?
            .json()
            .await
            .map_err(|_| upstream())?;
        if profile.login.is_empty() || profile.login.len() > 100 {
            return Err(upstream());
        }
        sqlx::query("INSERT INTO user_connections (user_id,provider,account_id,name) VALUES ($1,'github',$2,$3) ON CONFLICT (user_id,provider) DO UPDATE SET account_id=EXCLUDED.account_id,name=EXCLUDED.name,visible=FALSE")
            .bind(user).bind(profile.id.to_string()).bind(profile.login).execute(&self.pool).await.map_err(|_| AppError::Conflict { code: 0, message: "Account is already linked".into() })?;
        Ok(())
    }
    async fn list(&self, user: Snowflake, public_only: bool) -> AppResult<Vec<Connection>> {
        let rows: Vec<(String,String,String,bool)> = sqlx::query_as("SELECT provider,account_id,name,visible FROM user_connections WHERE user_id=$1 AND (NOT $2 OR visible) ORDER BY provider")
            .bind(user.as_i64()).bind(public_only).fetch_all(&self.pool).await.map_err(AppError::internal)?;
        Ok(rows
            .into_iter()
            .map(|(provider, account_id, name, visible)| Connection {
                provider,
                account_id,
                name,
                visible,
            })
            .collect())
    }
    async fn visibility(&self, user: Snowflake, visible: bool) -> AppResult<()> {
        let result = sqlx::query(
            "UPDATE user_connections SET visible=$2 WHERE user_id=$1 AND provider='github'",
        )
        .bind(user.as_i64())
        .bind(visible)
        .execute(&self.pool)
        .await
        .map_err(AppError::internal)?;
        if result.rows_affected() == 0 {
            return Err(AppError::invalid(
                "provider",
                "NOT_LINKED",
                "Account is not linked",
            ));
        }
        Ok(())
    }
    async fn delete(&self, user: Snowflake) -> AppResult<()> {
        sqlx::query("DELETE FROM user_connections WHERE user_id=$1 AND provider='github'")
            .bind(user.as_i64())
            .execute(&self.pool)
            .await
            .map_err(AppError::internal)?;
        Ok(())
    }
}
