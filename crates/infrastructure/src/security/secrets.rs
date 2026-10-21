//! Random secrets for tokens and invite codes, and deterministic token hashes.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::SigningKey;
use rand::distributions::Alphanumeric;
use rand::rngs::OsRng;
use rand::{Rng, RngCore};
use sha2::{Digest, Sha256};

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
