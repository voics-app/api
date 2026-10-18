//! Shared token issuance for register, login and refresh.

use uuid::Uuid;

use domain::user::User;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;
use crate::ports::{AccessClaims, Session};

/// 256-bit refresh tokens (P3).
pub const REFRESH_TOKEN_BYTES: usize = 32;

#[derive(Debug, Clone)]
pub struct AuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub user: User,
}

pub fn access_token(
    ports: &Ports,
    user_id: Snowflake,
    session_id: Uuid,
    device_id: Option<Snowflake>,
) -> AppResult<String> {
    let now = ports.clock.now();
    ports.tokens.issue(&AccessClaims {
        user_id,
        session_id,
        device_id,
        issued_at: now,
        expires_at: now + ports.settings.access_token_ttl,
    })
}

/// Starts a new session family for `user`.
pub async fn start_session(
    ports: &Ports,
    user: User,
    device_id: Option<Snowflake>,
) -> AppResult<AuthTokens> {
    let now = ports.clock.now();
    let session = Session {
        id: Uuid::new_v4(),
        user_id: user.id,
        device_id,
        created_at: now,
        revoked_at: None,
    };
    let refresh_token = ports.secrets.token(REFRESH_TOKEN_BYTES);
    let refresh_hash = ports.secrets.hash(&refresh_token);
    ports
        .sessions
        .create(
            &session,
            &refresh_hash,
            now + ports.settings.refresh_token_ttl,
        )
        .await?;
    let access_token = access_token(ports, user.id, session.id, device_id)?;
    Ok(AuthTokens {
        access_token,
        refresh_token,
        user,
    })
}
