//! Shared harness for integration tests: a throwaway Postgres container and
//! the production adapters wired into `Ports`. Only the messages service and
//! object storage are stubbed, because no test here reaches them.

#![allow(dead_code)]

pub mod http;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sqlx::PgPool;
use testcontainers::core::{ContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

use application::events::DomainEvent;
use application::ports::{AssetStorage, EventPublisher, MessageService, StoredMessage};

use application::{AppError, AppResult, Ports, Settings};
use infrastructure::postgres::{connect, migrate, PgStore, PgUnitOfWork};
use infrastructure::security::jwt::Ed25519Tokens;
use infrastructure::security::password::Argon2Passwords;
use infrastructure::security::secrets::OsSecrets;
use infrastructure::security::totp::Rfc6238;
use infrastructure::system::clock::SystemClock;
use infrastructure::system::ids::SnowflakeIds;

pub struct Database {
    /// Keeps the container alive for the duration of the test.
    pub _container: ContainerAsync<GenericImage>,
    pub pool: PgPool,
}

pub async fn start_database() -> Database {
    let container = GenericImage::new("postgres", "16-alpine")
        .with_exposed_port(ContainerPort::Tcp(5432))
        .with_wait_for(WaitFor::message_on_stderr(
            "database system is ready to accept connections",
        ))
        .with_env_var("POSTGRES_PASSWORD", "postgres")
        .with_env_var("POSTGRES_DB", "discordy")
        .start()
        .await
        .expect("start postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("mapped postgres port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/discordy");
    let mut pool = None;
    for _ in 0..60 {
        if let Ok(candidate) = connect(&url, 4).await {
            if sqlx::query("SELECT 1").execute(&candidate).await.is_ok() {
                pool = Some(candidate);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let pool = pool.expect("postgres accepts connections");
    migrate(&pool).await.expect("migrations apply");
    Database {
        _container: container,
        pool,
    }
}

/// Ed25519 key pair in PEM form, generated per test run.
pub fn jwt_keys() -> (Vec<u8>, Vec<u8>) {
    use ed25519_dalek::pkcs8::spki::der::pem::LineEnding;
    use ed25519_dalek::pkcs8::{EncodePrivateKey, EncodePublicKey};
    let signing = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
    let private = signing
        .to_pkcs8_pem(LineEnding::LF)
        .expect("pkcs8")
        .to_string();
    let public = signing
        .verifying_key()
        .to_public_key_pem(LineEnding::LF)
        .expect("spki");
    (private.into_bytes(), public.into_bytes())
}

#[derive(Default)]
struct DiscardEvents;

#[async_trait]
impl EventPublisher for DiscardEvents {
    async fn publish(&self, _event: DomainEvent) -> AppResult<()> {
        Ok(())
    }
}

struct NoMessages;

#[async_trait]
impl MessageService for NoMessages {
    async fn create(&self, _: domain::Snowflake, _: serde_json::Value) -> AppResult<StoredMessage> {
        Err(AppError::Unavailable(
            "messages service not wired in this test".into(),
        ))
    }
    async fn edit(
        &self,
        _: domain::Snowflake,
        _: domain::Snowflake,
        _: serde_json::Value,
    ) -> AppResult<StoredMessage> {
        Err(AppError::Unavailable(
            "messages service not wired in this test".into(),
        ))
    }
    async fn delete(&self, _: domain::Snowflake, _: domain::Snowflake) -> AppResult<()> {
        Err(AppError::Unavailable(
            "messages service not wired in this test".into(),
        ))
    }
}

/// In-memory object store standing in for MinIO.
#[derive(Default)]
struct MemoryAssets {
    objects: std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>,
}

#[async_trait]
impl AssetStorage for MemoryAssets {
    async fn put(&self, key: &str, bytes: Vec<u8>, _content_type: &str) -> AppResult<()> {
        self.objects
            .lock()
            .expect("asset store lock")
            .insert(key.to_owned(), bytes);
        Ok(())
    }
}

/// Builds the application ports over `pool` with the production adapters.
pub fn ports(pool: &PgPool, public_pem: &[u8], private_pem: &[u8]) -> Ports {
    let store = Arc::new(PgStore::new(pool.clone()));
    Ports {
        users: store.clone(),
        sessions: store.clone(),
        devices: store.clone(),
        relationships: store.clone(),
        channels: store.clone(),
        guilds: store.clone(),
        invites: store.clone(),
        emojis: store.clone(),
        audit_log: store.clone(),
        applications: store.clone(),
        interactions: store.clone(),
        webhooks: store,
        unit_of_work: Arc::new(PgUnitOfWork::new(pool.clone(), "nl-home".into())),
        events: Arc::new(DiscardEvents),
        passwords: Arc::new(Argon2Passwords),
        tokens: Arc::new(Ed25519Tokens::from_pem(private_pem, public_pem).expect("jwt keys")),
        clock: Arc::new(SystemClock),
        ids: Arc::new(SnowflakeIds::new(1).expect("worker id")),
        secrets: Arc::new(OsSecrets),
        totp: Arc::new(Rfc6238),
        assets: Arc::new(MemoryAssets::default()),
        messages: Arc::new(NoMessages),
        settings: Settings::default(),
    }
}
