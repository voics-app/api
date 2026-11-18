//! Random secrets for tokens and invite codes, and deterministic token hashes.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::SigningKey;
use hmac::{Hmac, Mac};
use rand::distributions::Alphanumeric;
use rand::rngs::OsRng;
use rand::{Rng, RngCore};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use application::ports::SecretGenerator;
use domain::invite::INVITE_CODE_LENGTH;

#[derive(Debug, Default, Clone, Copy)]
pub struct OsSecrets;

impl SecretGenerator for OsSecrets {
    fn token(&self, bytes: usize) -> String {
        let mut buffer = vec![0_u8; bytes];
        OsRng.fill_bytes(&mut buffer);
        URL_SAFE_NO_PAD.encode(buffer)
    }

    fn retryable_refresh_token(&self, previous: &str, request_id: Uuid) -> String {
        // HMAC accepts keys of any length; old refresh tokens carry 256 bits of
        // entropy. Domain separation prevents this derivation being reused for
        // another token purpose. UUID bytes normalize equivalent spellings.
        let mut mac = Hmac::<Sha256>::new_from_slice(previous.as_bytes())
            .expect("HMAC accepts arbitrary key lengths");
        mac.update(b"voics.refresh.successor.v1\0");
        mac.update(request_id.as_bytes());
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }

    fn invite_code(&self) -> String {
        OsRng
            .sample_iter(&Alphanumeric)
            .take(INVITE_CODE_LENGTH)
            .map(char::from)
            .collect()
    }

    fn backup_code(&self) -> String {
        const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
        let mut rng = OsRng;
        (0..domain::mfa::BACKUP_CODE_LENGTH)
            .map(|_| char::from(ALPHABET[rng.gen_range(0..ALPHABET.len())]))
            .collect()
    }

    fn hash(&self, secret: &str) -> String {
        hex::encode(Sha256::digest(secret.as_bytes()))
    }

    fn verify_key(&self) -> String {
        let signing = SigningKey::generate(&mut OsRng);
        hex::encode(signing.verifying_key().to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_refresh_tokens_are_deterministic_and_bound_to_both_inputs() {
        let request = Uuid::new_v4();
        let token = OsSecrets.retryable_refresh_token("old-secret", request);
        assert_eq!(URL_SAFE_NO_PAD.decode(&token).unwrap().len(), 32);
        assert_eq!(
            token,
            OsSecrets.retryable_refresh_token("old-secret", request)
        );
        assert_ne!(
            token,
            OsSecrets.retryable_refresh_token("other-secret", request)
        );
        assert_ne!(
            token,
            OsSecrets.retryable_refresh_token("old-secret", Uuid::new_v4())
        );
    }

    #[test]
    fn tokens_have_the_requested_entropy_and_encoding() {
        let token = OsSecrets.token(32);
        assert_eq!(URL_SAFE_NO_PAD.decode(&token).unwrap().len(), 32);
        assert_ne!(token, OsSecrets.token(32));
    }

    #[test]
    fn hashes_are_deterministic_and_hex() {
        assert_eq!(OsSecrets.hash("abc"), OsSecrets.hash("abc"));
        assert_eq!(OsSecrets.hash("abc").len(), 64);
    }

    #[test]
    fn invite_codes_are_alphanumeric() {
        let code = OsSecrets.invite_code();
        assert_eq!(code.len(), INVITE_CODE_LENGTH);
        assert!(code.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn backup_codes_are_canonical() {
        let code = OsSecrets.backup_code();
        assert_eq!(domain::mfa::normalize_backup_code(&code), Some(code));
    }

    #[test]
    fn verify_keys_are_32_byte_hex() {
        assert_eq!(OsSecrets.verify_key().len(), 64);
    }
}
