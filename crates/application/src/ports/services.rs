//! Technical service ports implemented by infrastructure adapters.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use domain::Snowflake;

use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::write_set::WriteSet;

/// Applies a write set atomically, outbox events included.
#[async_trait]
pub trait UnitOfWork: Send + Sync {
    async fn commit(&self, writes: WriteSet) -> AppResult<()>;
}

/// Publishes events that need no transactional guarantee (e.g. typing).
#[async_trait]
pub trait EventPublisher: Send + Sync {
    async fn publish(&self, event: DomainEvent) -> AppResult<()>;
}

#[async_trait]
pub trait PasswordHasher: Send + Sync {
    async fn hash(&self, password: &str) -> AppResult<String>;
    async fn verify(&self, password: &str, hash: &str) -> AppResult<bool>;
}

/// Claims of a user access token (P3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessClaims {
    pub user_id: Snowflake,
    pub session_id: Uuid,
    pub device_id: Option<Snowflake>,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

pub trait TokenIssuer: Send + Sync {
    fn issue(&self, claims: &AccessClaims) -> AppResult<String>;
    /// Verifies signature, issuer, kind and expiry.
    fn verify(&self, token: &str) -> AppResult<AccessClaims>;
}

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub trait IdGenerator: Send + Sync {
    fn next_id(&self) -> AppResult<Snowflake>;
    fn is_ready(&self) -> bool;
}

#[async_trait]
pub trait IdReservations: Send + Sync {
    async fn reserve(
        &self,
        request: voics_snowflake::ReservationRequest,
    ) -> AppResult<voics_snowflake::Reservation>;
}

/// Random secrets and their one-way hashes.
pub trait SecretGenerator: Send + Sync {
    /// `bytes` of randomness, base64url without padding.
    fn token(&self, bytes: usize) -> String;
    /// A deterministic 256-bit successor for an idempotent refresh. The previous
    /// bearer token is the HMAC key; neither input is persisted by this service.
    fn retryable_refresh_token(&self, previous: &str, request_id: Uuid) -> String;
    /// Alphanumeric invite code.
    fn invite_code(&self) -> String;
    /// Deterministic one-way hash used to store tokens.
    fn hash(&self, secret: &str) -> String;
    /// A fresh backup code in canonical form (see `domain::mfa`).
    fn backup_code(&self) -> String;
    /// Hex-encoded Ed25519 public key for a new application (`verify_key`).
    fn verify_key(&self) -> String;
}

pub trait TotpVerifier: Send + Sync {
    fn verify(&self, secret: &str, code: &str, now: DateTime<Utc>) -> bool;
    fn generate_secret(&self) -> String;
}

#[async_trait]
pub trait AssetStorage: Send + Sync {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> AppResult<()>;
}

/// A message as stored by the `messages` service.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredMessage {
    pub id: Snowflake,
    /// The full Discord-shaped message object.
    pub body: Value,
}

/// Messages are owned by the `messages` service.
#[async_trait]
pub trait MessageService: Send + Sync {
    /// Observed message markers and whether this UTC day has verified coverage.
    async fn guild_message_count(
        &self,
        _guild_id: Snowflake,
        _utc_day: i64,
    ) -> AppResult<(u64, bool, Option<u64>)> {
        Err(AppError::Unavailable(
            "guild message activity is not configured".into(),
        ))
    }
    /// Server-stored forum summaries for already authorized channels.
    async fn forum_summaries(&self, _channel_ids: &[Snowflake]) -> AppResult<Vec<Value>> {
        Err(AppError::Unavailable("forum summaries unavailable".into()))
    }
    /// One invocation only after the caller durably claims the reserved forum message.
    async fn create_forum_start(
        &self,
        _channel_id: Snowflake,
        _message: Value,
    ) -> AppResult<StoredMessage> {
        Err(AppError::Unavailable("forum start is unavailable".into()))
    }

    /// Creates a message on behalf of a bot, webhook or interaction.
    async fn create(&self, channel_id: Snowflake, message: Value) -> AppResult<StoredMessage>;
    async fn edit(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        patch: Value,
    ) -> AppResult<StoredMessage>;
    async fn delete(&self, channel_id: Snowflake, message_id: Snowflake) -> AppResult<()>;
    /// One stored message; `None` when it does not exist (or is not stored, as for ephemeral
    /// and P2P chats).
    async fn get(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> AppResult<Option<StoredMessage>>;
    async fn mark_crossposted(
        &self,
        _channel_id: Snowflake,
        _message_id: Snowflake,
    ) -> AppResult<StoredMessage> {
        Err(AppError::Unavailable(
            "message publication marker unavailable".into(),
        ))
    }
    /// Sends a journalled follower delivery. Errors after dispatch may be ambiguous.
    async fn deliver_crosspost(
        &self,
        _channel_id: Snowflake,
        _body: Value,
    ) -> AppResult<StoredMessage> {
        Err(AppError::Unavailable(
            "crosspost delivery unavailable".into(),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateLimitDecision {
    pub allowed: bool,
    pub limit: u32,
    pub remaining: u32,
    pub reset_after: Duration,
    /// How long a rejected caller must wait before the next token is available.
    pub retry_after: Duration,
}

/// Live presence, owned by the gateway. Read-only here: the api only counts.
#[async_trait]
pub trait PresenceDirectory: Send + Sync {
    /// How many of `user_ids` are currently online (any status but offline/invisible).
    async fn online_count(&self, user_ids: &[Snowflake]) -> AppResult<u64>;
    /// Which of `user_ids` hold at least one live gateway session, whatever their status.
    async fn live_ids(&self, user_ids: &[Snowflake]) -> AppResult<Vec<Snowflake>>;
    /// The gateway owns voice state; inability to check never grants permission.
    async fn in_voice_channel(
        &self,
        _guild_id: Snowflake,
        _channel_id: Snowflake,
        _user_id: Snowflake,
    ) -> AppResult<bool> {
        Err(AppError::Unavailable(
            "voice state lookup is not configured".into(),
        ))
    }
}

#[async_trait]
pub trait RateLimiter: Send + Sync {
    async fn hit(&self, key: &str, limit: u32, window: Duration) -> AppResult<RateLimitDecision>;
}

/// A plain-text email.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    pub to: String,
    pub subject: String,
    pub body: String,
}

#[async_trait]
pub trait Mailer: Send + Sync {
    async fn send(&self, mail: &Mail) -> AppResult<()>;
}
