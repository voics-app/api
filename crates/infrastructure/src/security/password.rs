//! Argon2id password hashing, run off the async executor.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use argon2::password_hash::{
    rand_core::{OsRng, RngCore},
    Output, ParamsString, PasswordHash, Salt, SaltString,
};
use argon2::{Algorithm, Argon2, Block, Params, Version};
use async_trait::async_trait;

use application::ports::PasswordHasher;
use application::{AppError, AppResult};

use super::gate::BlockingGate;

const SALT_LEN: usize = 16;

/// Argon2id costs tens of MB per call, so every hash and verify goes through
/// one [`BlockingGate`]; without it a few parallel logins exhaust the pod.
///
/// The memory matrix (19 MiB at the default cost) is also reused instead of
/// allocated per call. With a fresh allocation each time, glibc malloc moves
/// the 19 MiB blocks into per-thread arenas after the first free and keeps
/// them there fragmented: sequential registrations alone took the api from
/// 11 MiB to over 300 MiB RSS (api#119). A gate permit holder takes a matrix
/// from the pool and returns it, so at most `concurrency` matrices ever exist.
#[derive(Debug, Clone)]
pub struct Argon2Passwords {
    gate: BlockingGate,
    matrices: Arc<MatrixPool>,
}

impl Argon2Passwords {
    pub fn new(concurrency: usize, queue_timeout: Duration) -> Self {
        Self {
            gate: BlockingGate::new(concurrency, queue_timeout),
            matrices: Arc::default(),
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
        let matrices = self.matrices.clone();
        self.gate
            .run(move || {
                let argon2 = Argon2::default();
                let mut raw_salt = [0u8; SALT_LEN];
                OsRng.fill_bytes(&mut raw_salt);
                let salt = SaltString::encode_b64(&raw_salt).map_err(AppError::internal)?;
                let digest = matrices.derive(&argon2, password.as_bytes(), &raw_salt)?;
                let phc = PasswordHash {
                    algorithm: Algorithm::Argon2id.ident(),
                    version: Some(Version::V0x13.into()),
                    params: ParamsString::try_from(argon2.params()).map_err(AppError::internal)?,
                    salt: Some(salt.as_salt()),
                    hash: Some(digest),
                };
                Ok(phc.to_string())
            })
            .await
    }

    async fn verify(&self, password: &str, hash: &str) -> AppResult<bool> {
        let password = password.to_owned();
        let hash = hash.to_owned();
        let matrices = self.matrices.clone();
        self.gate
            .run(move || {
                let parsed = PasswordHash::new(&hash).map_err(AppError::internal)?;
                let (Some(salt), Some(expected)) = (parsed.salt, parsed.hash) else {
                    return Ok(false);
                };
                let Ok(algorithm) = Algorithm::try_from(parsed.algorithm) else {
                    return Ok(false);
                };
                let version = match parsed.version.map(Version::try_from) {
                    None => Version::default(),
                    Some(Ok(version)) => version,
                    Some(Err(_)) => return Ok(false),
                };
                let Ok(params) = Params::try_from(&parsed) else {
                    return Ok(false);
                };
                let mut salt_buf = [0u8; Salt::MAX_LENGTH];
                let Ok(raw_salt) = salt.decode_b64(&mut salt_buf) else {
                    return Ok(false);
                };
                let argon2 = Argon2::new(algorithm, version, params);
                Ok(matrices
                    .derive(&argon2, password.as_bytes(), raw_salt)
                    .is_ok_and(|digest| digest == expected))
            })
            .await
    }
}

/// Idle memory matrices. Callers hold a gate permit, so the pool never holds
/// more matrices than there are permits.
#[derive(Debug, Default)]
struct MatrixPool {
    idle: Mutex<Vec<Box<[Block]>>>,
}

impl MatrixPool {
    /// Runs one argon2 derivation (32-byte output) over a pooled matrix.
    fn derive(&self, argon2: &Argon2<'_>, password: &[u8], salt: &[u8]) -> AppResult<Output> {
        let mut matrix = self.take(argon2.params().block_count());
        let mut out = [0u8; Params::DEFAULT_OUTPUT_LEN];
        let derived =
            argon2.hash_password_into_with_memory(password, salt, &mut out, &mut matrix[..]);
        self.give_back(matrix);
        derived.map_err(AppError::internal)?;
        Output::new(&out).map_err(AppError::internal)
    }

    /// A matrix of exactly `blocks` blocks; a pooled one of another size (the
    /// cost parameters changed) is dropped.
    fn take(&self, blocks: usize) -> Box<[Block]> {
        let pooled = self
            .idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
            .filter(|matrix| matrix.len() == blocks);
        pooled.unwrap_or_else(|| vec![Block::default(); blocks].into_boxed_slice())
    }

    fn give_back(&self, matrix: Box<[Block]>) {
        self.idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(matrix);
    }

    #[cfg(test)]
    fn idle_matrices(&self) -> usize {
        self.idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

#[cfg(test)]
mod tests {
    use argon2::password_hash::{PasswordHasher as _, PasswordVerifier as _};

    use super::*;

    #[tokio::test]
    async fn hashes_verify_and_reject_wrong_passwords() {
        let hasher = Argon2Passwords::default();
        let hash = hasher.hash("correct horse").await.unwrap();
        assert!(hasher.verify("correct horse", &hash).await.unwrap());
        assert!(!hasher.verify("battery staple", &hash).await.unwrap());
    }

    #[tokio::test]
    async fn hashes_are_standard_argon2id_phc_strings_with_unchanged_cost() {
        let hasher = Argon2Passwords::default();
        let hash = hasher.hash("correct horse").await.unwrap();
        assert!(
            hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
            "cost parameters changed: {hash}"
        );
        let parsed = PasswordHash::new(&hash).unwrap();
        assert!(Argon2::default()
            .verify_password(b"correct horse", &parsed)
            .is_ok());
        assert!(Argon2::default()
            .verify_password(b"battery staple", &parsed)
            .is_err());
    }

    #[tokio::test]
    async fn verifies_hashes_made_by_the_stock_argon2_implementation() {
        let salt = SaltString::generate(&mut OsRng);
        let stock = Argon2::default()
            .hash_password(b"correct horse", &salt)
            .unwrap()
            .to_string();
        let hasher = Argon2Passwords::default();
        assert!(hasher.verify("correct horse", &stock).await.unwrap());
        assert!(!hasher.verify("battery staple", &stock).await.unwrap());
    }

    #[tokio::test]
    async fn verifies_hashes_stored_with_other_cost_parameters() {
        let params = Params::new(8, 1, 1, None).unwrap();
        let cheap = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let salt = SaltString::generate(&mut OsRng);
        let stored = cheap
            .hash_password(b"correct horse", &salt)
            .unwrap()
            .to_string();
        let hasher = Argon2Passwords::default();
        assert!(hasher.verify("correct horse", &stored).await.unwrap());
        assert!(!hasher.verify("battery staple", &stored).await.unwrap());
        let hash = hasher.hash("correct horse").await.unwrap();
        assert!(hasher.verify("correct horse", &hash).await.unwrap());
    }

    #[tokio::test]
    async fn a_hash_that_is_not_phc_is_an_error_and_a_foreign_algorithm_never_matches() {
        let hasher = Argon2Passwords::default();
        assert!(hasher.verify("x", "not a hash").await.is_err());
        assert!(!hasher
            .verify(
                "x",
                "$scrypt$ln=4,r=8,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaGhhc2g"
            )
            .await
            .unwrap());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn many_hashes_reuse_at_most_one_matrix_per_gate_permit() {
        let hasher = Argon2Passwords::new(2, Duration::from_secs(60));
        let mut tasks = Vec::new();
        for n in 0..12 {
            let hasher = hasher.clone();
            tasks.push(tokio::spawn(async move {
                let password = format!("password {n}");
                let hash = hasher.hash(&password).await.unwrap();
                assert!(hasher.verify(&password, &hash).await.unwrap());
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert!(
            hasher.matrices.idle_matrices() <= 2,
            "{} matrices allocated for 2 permits",
            hasher.matrices.idle_matrices()
        );
    }
}
