//! Argon2id password hashing, run off the async executor.

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher as _, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use async_trait::async_trait;

use std::time::Duration;

use application::ports::PasswordHasher;
use application::{AppError, AppResult};

use super::gate::BlockingGate;

/// Argon2id costs tens of MB per call, so every hash and verify goes through
/// one [`BlockingGate`]; without it a few parallel logins exhaust the pod.
#[derive(Debug, Clone)]
pub struct Argon2Passwords {
    gate: BlockingGate,
}

impl Argon2Passwords {
    pub fn new(concurrency: usize, queue_timeout: Duration) -> Self {
        Self {
            gate: BlockingGate::new(concurrency, queue_timeout),
        }
    }
}

impl Default for Argon2Passwords {
    fn default() -> Self {
        Self::new(2, Duration::from_secs(5))
    }
}

#[async_trait]
impl PasswordHasher for Argon2Passwords {
    async fn hash(&self, password: &str) -> AppResult<String> {
        let password = password.to_owned();
        self.gate
            .run(move || {
                let salt = SaltString::generate(&mut OsRng);
                Argon2::default()
                    .hash_password(password.as_bytes(), &salt)
                    .map(|hash| hash.to_string())
                    .map_err(AppError::internal)
            })
            .await
    }

    async fn verify(&self, password: &str, hash: &str) -> AppResult<bool> {
        let password = password.to_owned();
        let hash = hash.to_owned();
        self.gate
            .run(move || {
                let parsed = PasswordHash::new(&hash).map_err(AppError::internal)?;
                Ok(Argon2::default()
                    .verify_password(password.as_bytes(), &parsed)
                    .is_ok())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hashes_verify_and_reject_wrong_passwords() {
        let hasher = Argon2Passwords::default();
        let hash = hasher.hash("correct horse").await.unwrap();
        assert!(hasher.verify("correct horse", &hash).await.unwrap());
        assert!(!hasher.verify("battery staple", &hash).await.unwrap());
    }
}
