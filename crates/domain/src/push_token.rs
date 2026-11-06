//! Push tokens of the mobile app: one APNs or FCM token per login session.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::Snowflake;

/// Most push tokens a user keeps; registering one more drops the least recently refreshed.
pub const MAX_PUSH_TOKENS_PER_USER: usize = 20;
/// APNs device tokens are 32 bytes today (64 hex characters) and Apple allows them to grow.
const APNS_TOKEN_MAX_CHARS: usize = 200;
/// FCM registration tokens are about 160 characters; the documented ceiling is generous.
const FCM_TOKEN_MAX_CHARS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushPlatform {
    Apns,
    Fcm,
}

impl PushPlatform {
    pub const fn as_str(self) -> &'static str {
        match self {
            PushPlatform::Apns => "apns",
            PushPlatform::Fcm => "fcm",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "apns" => Some(PushPlatform::Apns),
            "fcm" => Some(PushPlatform::Fcm),
            _ => None,
        }
    }
}

/// Which APNs host a token belongs to: development builds get sandbox tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushEnvironment {
    Production,
    Sandbox,
}

impl PushEnvironment {
    pub const fn as_str(self) -> &'static str {
        match self {
            PushEnvironment::Production => "production",
            PushEnvironment::Sandbox => "sandbox",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "production" => Some(PushEnvironment::Production),
            "sandbox" => Some(PushEnvironment::Sandbox),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushToken {
    pub session_id: Uuid,
    pub user_id: Snowflake,
    pub device_id: Option<Snowflake>,
    pub platform: PushPlatform,
    pub token: String,
    pub environment: PushEnvironment,
    /// Whether pushes of standard messages may carry a text preview.
    pub previews: bool,
    /// The device wants pushes for mentions and direct messages only, whatever the server settings say.
    pub only_mentions: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The PushKit (VoIP) token of an iPhone session, which only rings the phone for a call. APNs only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoipToken {
    pub session_id: Uuid,
    pub user_id: Snowflake,
    pub token: String,
    pub environment: PushEnvironment,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PushTokenError {
    #[error("the token must not be empty")]
    Empty,
    #[error("the token is too long for its platform")]
    TooLong,
    #[error("an APNs token is hexadecimal")]
    NotHex,
}

/// Checks the shape of a provider token. Whether the provider accepts it is only known at send
/// time, where a rejected token is removed again.
pub fn validate_token(platform: PushPlatform, token: &str) -> Result<(), PushTokenError> {
    if token.is_empty() {
        return Err(PushTokenError::Empty);
    }
    match platform {
        PushPlatform::Apns if token.len() > APNS_TOKEN_MAX_CHARS => Err(PushTokenError::TooLong),
        PushPlatform::Apns if !token.chars().all(|c| c.is_ascii_hexdigit()) => {
            Err(PushTokenError::NotHex)
        }
        PushPlatform::Fcm if token.len() > FCM_TOKEN_MAX_CHARS => Err(PushTokenError::TooLong),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apns_tokens_are_bounded_hex() {
        assert_eq!(validate_token(PushPlatform::Apns, &"ab".repeat(32)), Ok(()));
        assert_eq!(
            validate_token(PushPlatform::Apns, ""),
            Err(PushTokenError::Empty)
        );
        assert_eq!(
            validate_token(PushPlatform::Apns, "not-hex"),
            Err(PushTokenError::NotHex)
        );
        assert_eq!(
            validate_token(PushPlatform::Apns, &"a".repeat(201)),
            Err(PushTokenError::TooLong)
        );
    }

    #[test]
    fn fcm_tokens_are_free_form_but_bounded() {
        assert_eq!(validate_token(PushPlatform::Fcm, "abc:APA91b-_x"), Ok(()));
        assert_eq!(
            validate_token(PushPlatform::Fcm, &"a".repeat(4097)),
            Err(PushTokenError::TooLong)
        );
    }

    #[test]
    fn platform_and_environment_round_trip() {
        for platform in [PushPlatform::Apns, PushPlatform::Fcm] {
            assert_eq!(PushPlatform::parse(platform.as_str()), Some(platform));
        }
        for env in [PushEnvironment::Production, PushEnvironment::Sandbox] {
            assert_eq!(PushEnvironment::parse(env.as_str()), Some(env));
        }
        assert_eq!(PushPlatform::parse("wns"), None);
        assert_eq!(PushEnvironment::parse("staging"), None);
    }
}
