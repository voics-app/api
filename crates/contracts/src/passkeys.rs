//! Additive WebAuthn shapes, mirrored from contracts REST schemas.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct RegistrationStart {
    pub password: String,
    pub name: String,
}

#[derive(Deserialize)]
pub struct RegistrationFinish {
    pub challenge_id: Uuid,
    pub credential: Value,
}

#[derive(Deserialize)]
pub struct RemovePasskey {
    pub password: String,
    pub code: String,
}

#[derive(Deserialize)]
pub struct AuthenticationStart {
    pub ticket: String,
}

#[derive(Deserialize)]
pub struct AuthenticationFinish {
    pub ticket: String,
    pub challenge_id: Uuid,
    pub credential: Value,
}

#[derive(Serialize)]
pub struct Challenge {
    pub challenge_id: Uuid,
    #[serde(rename = "publicKey")]
    pub public_key: Value,
}

#[derive(Serialize)]
pub struct Passkey {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
pub struct BackupCode {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub consumed: bool,
}

#[derive(Serialize)]
pub struct RegisteredPasskey {
    pub passkey: Passkey,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_codes: Option<Vec<BackupCode>>,
}
