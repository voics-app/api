//! Passkey storage and cryptographic ceremonies. Opaque verifier state never reaches clients.

use crate::AppResult;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::Snowflake;
use serde_json::Value;
use uuid::Uuid;

pub const MAX_PASSKEYS: usize = 10;
pub const CHALLENGE_LIFETIME_SECONDS: i64 = 300;

#[derive(Debug, Clone)]
pub struct PasskeyRecord {
    pub id: Uuid,
    pub user_id: Snowflake,
    pub name: String,
    pub credential: VerifiedCredential,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct VerifiedCredential {
    pub id: Vec<u8>,
    pub material: Value,
    pub counter: u32,
}

#[derive(Debug, Clone)]
pub struct Ceremony {
    pub public_key: Value,
    pub state: Value,
}

#[derive(Debug, Clone)]
pub struct PasskeyChallenge {
    pub id: Uuid,
    pub user_id: Snowflake,
    pub kind: ChallengeKind,
    /// Session UUID for enrollment; hashed password-authenticated ticket for login.
    pub binding: String,
    pub name: Option<String>,
    pub state: Value,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy)]
pub enum ChallengeKind {
    Registration,
    Authentication,
}
impl ChallengeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Registration => "registration",
            Self::Authentication => "authentication",
        }
    }
}

pub trait WebauthnVerifier: Send + Sync {
    fn start_registration(
        &self,
        user: &domain::user::User,
        existing: &[PasskeyRecord],
    ) -> AppResult<Ceremony>;
    fn finish_registration(&self, response: Value, state: Value) -> AppResult<VerifiedCredential>;
    fn start_authentication(&self, credentials: &[PasskeyRecord]) -> AppResult<Ceremony>;
    fn finish_authentication(
        &self,
        response: Value,
        state: Value,
        credentials: &[PasskeyRecord],
    ) -> AppResult<VerifiedCredential>;
}

#[async_trait]
pub trait PasskeyRepository: Send + Sync {
    async fn list(&self, user_id: Snowflake) -> AppResult<Vec<PasskeyRecord>>;
    async fn create_challenge(
        &self,
        challenge: &PasskeyChallenge,
        now: DateTime<Utc>,
    ) -> AppResult<()>;
    /// Atomically removes an unexpired, correctly-bound ceremony, including failed assertions.
    async fn take_challenge(
        &self,
        id: Uuid,
        kind: ChallengeKind,
        binding: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<PasskeyChallenge>>;
    /// Atomically enforces the per-account limit, saves a credential and enables MFA.
    /// Initial recovery hashes are inserted only when no recovery codes exist yet.
    async fn register(&self, record: &PasskeyRecord, recovery_hashes: &[String])
        -> AppResult<bool>;
    /// Counters must advance if either stored/new counter is nonzero. Ticket consumption and
    /// credential update commit together; concurrent assertions cannot reuse either proof.
    async fn authenticate(
        &self,
        user_id: Snowflake,
        credential: &VerifiedCredential,
        ticket_hash: &str,
        now: DateTime<Utc>,
    ) -> AppResult<bool>;
    async fn remove(&self, user_id: Snowflake, id: Uuid) -> AppResult<bool>;
}
