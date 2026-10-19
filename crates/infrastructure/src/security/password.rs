//! Argon2id password hashing, run off the async executor.

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher as _, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use async_trait::async_trait;

use application::ports::PasswordHasher;
use application::{AppError, AppResult};

#[derive(Debug, Default, Clone, Copy)]
pub struct Argon2Passwords;

#[async_trait]
impl PasswordHasher for Argon2Passwords {
    async fn hash(&self, password: &str) -> AppResult<String> {
        let password = password.to_owned();
        tokio::task::spawn_blocking(move || {
            let salt = SaltString::generate(&mut OsRng);
            Argon2::default()
                .hash_password(password.as_bytes(), &salt)
                .map(|hash| hash.to_string())
                .map_err(AppError::internal)
        })
        .await
        .map_err(AppError::internal)?
    }

    async fn verify(&self, password: &str, hash: &str) -> AppResult<bool> {
        let password = password.to_owned();
        let hash = hash.to_owned();
        tokio::task::spawn_blocking(move || {
            let parsed = PasswordHash::new(&hash).map_err(AppError::internal)?;
            Ok(Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok())
        })
        .await
        .map_err(AppError::internal)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hashes_verify_and_reject_wrong_passwords() {
        let hasher = Argon2Passwords;
        let hash = hasher.hash("correct horse").await.unwrap();
        assert!(hasher.verify("correct horse", &hash).await.unwrap());
        assert!(!hasher.verify("battery staple", &hash).await.unwrap());
    }
}
