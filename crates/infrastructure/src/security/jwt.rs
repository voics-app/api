//! Ed25519 (EdDSA) user access tokens, P3.

use chrono::{DateTime, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use application::ports::{AccessClaims, TokenIssuer};
use application::{AppError, AppResult};
use domain::Snowflake;

pub const ISSUER: &str = "discordy-api";
const USER_KIND: &str = "user";

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    sid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    did: Option<String>,
    kind: String,
    iat: i64,
    exp: i64,
}

pub struct Ed25519Tokens {
    encoding: EncodingKey,
    decoding: DecodingKey,
}

impl Ed25519Tokens {
    /// Builds the issuer from PEM-encoded Ed25519 keys (P2: `JWT_PRIVATE_KEY`, `JWT_PUBLIC_KEY`).
    pub fn from_pem(private_pem: &[u8], public_pem: &[u8]) -> AppResult<Self> {
        Ok(Self {
            encoding: EncodingKey::from_ed_pem(private_pem).map_err(AppError::internal)?,
            decoding: DecodingKey::from_ed_pem(public_pem).map_err(AppError::internal)?,
        })
    }
}

fn validation() -> Validation {
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.set_issuer(&[ISSUER]);
    validation.leeway = 0;
    validation
}

fn verify_with(decoding: &DecodingKey, token: &str) -> AppResult<AccessClaims> {
    let data =
        decode::<Claims>(token, decoding, &validation()).map_err(|_| AppError::Unauthorized)?;
    let claims = data.claims;
    if claims.kind != USER_KIND {
        return Err(AppError::Unauthorized);
    }
    Ok(AccessClaims {
        user_id: parse_snowflake(&claims.sub)?,
        session_id: Uuid::parse_str(&claims.sid).map_err(|_| AppError::Unauthorized)?,
        device_id: claims.did.as_deref().map(parse_snowflake).transpose()?,
        issued_at: timestamp(claims.iat)?,
        expires_at: timestamp(claims.exp)?,
    })
}

fn parse_snowflake(raw: &str) -> AppResult<Snowflake> {
    raw.parse().map_err(|_| AppError::Unauthorized)
}

fn timestamp(seconds: i64) -> AppResult<DateTime<Utc>> {
    DateTime::from_timestamp(seconds, 0).ok_or(AppError::Unauthorized)
}

impl TokenIssuer for Ed25519Tokens {
    fn issue(&self, claims: &AccessClaims) -> AppResult<String> {
        let body = Claims {
            iss: ISSUER.to_owned(),
            sub: claims.user_id.to_string(),
            sid: claims.session_id.to_string(),
            did: claims.device_id.map(|id| id.to_string()),
            kind: USER_KIND.to_owned(),
            iat: claims.issued_at.timestamp(),
            exp: claims.expires_at.timestamp(),
        };
        encode(&Header::new(Algorithm::EdDSA), &body, &self.encoding).map_err(AppError::internal)
    }

    fn verify(&self, token: &str) -> AppResult<AccessClaims> {
        verify_with(&self.decoding, token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::pkcs8::spki::der::pem::LineEnding;
    use ed25519_dalek::pkcs8::{EncodePrivateKey, EncodePublicKey};
    use ed25519_dalek::SigningKey;

    fn key_pair() -> (String, String) {
        let signing = SigningKey::generate(&mut rand::rngs::OsRng);
        let private = signing.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
        let public = signing
            .verifying_key()
            .to_public_key_pem(LineEnding::LF)
            .unwrap();
        (private, public)
    }

    fn claims(now: DateTime<Utc>) -> AccessClaims {
        AccessClaims {
            user_id: Snowflake::new(10),
            session_id: Uuid::nil(),
            device_id: Some(Snowflake::new(20)),
            issued_at: now,
            expires_at: now + chrono::Duration::minutes(15),
        }
    }

    fn whole_seconds_now() -> DateTime<Utc> {
        DateTime::from_timestamp(Utc::now().timestamp(), 0).unwrap()
    }

    #[test]
    fn issued_tokens_verify_and_carry_the_claims() {
        let (private, public) = key_pair();
        let tokens = Ed25519Tokens::from_pem(private.as_bytes(), public.as_bytes()).unwrap();
        let now = whole_seconds_now();
        let token = tokens.issue(&claims(now)).unwrap();
        assert_eq!(tokens.verify(&token).unwrap(), claims(now));
    }

    #[test]
    fn expired_tokens_are_rejected() {
        let (private, public) = key_pair();
        let tokens = Ed25519Tokens::from_pem(private.as_bytes(), public.as_bytes()).unwrap();
        let long_ago = whole_seconds_now() - chrono::Duration::hours(1);
        let token = tokens.issue(&claims(long_ago)).unwrap();
        assert!(matches!(tokens.verify(&token), Err(AppError::Unauthorized)));
    }

    #[test]
    fn tokens_signed_by_another_key_are_rejected() {
        let (private, _) = key_pair();
        let (_, other_public) = key_pair();
        let issuer = Ed25519Tokens::from_pem(private.as_bytes(), other_public.as_bytes()).unwrap();
        let token = issuer.issue(&claims(whole_seconds_now())).unwrap();
        assert!(matches!(issuer.verify(&token), Err(AppError::Unauthorized)));
    }
}
