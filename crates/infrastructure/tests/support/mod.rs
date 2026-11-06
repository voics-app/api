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
        .with_startup_timeout(Duration::from_secs(300))
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

/// Records the message bodies api sends to the `messages` service.
#[derive(Default)]
pub struct RecordingMessages {
    pub bodies: std::sync::Mutex<Vec<serde_json::Value>>,
    /// The channel each recorded body was posted to.
    pub channels: std::sync::Mutex<Vec<domain::Snowflake>>,
    /// Messages `get` serves, keyed by channel and message id.
    pub stored: std::sync::Mutex<
        std::collections::HashMap<(domain::Snowflake, domain::Snowflake), serde_json::Value>,
    >,
}

#[async_trait]
impl MessageService for RecordingMessages {
    async fn create(
        &self,
        channel_id: domain::Snowflake,
        message: serde_json::Value,
    ) -> AppResult<StoredMessage> {
        self.channels.lock().unwrap().push(channel_id);
        self.bodies.lock().unwrap().push(message);
        Ok(StoredMessage {
            id: domain::Snowflake::new(555),
            body: serde_json::json!({"id": "555"}),
        })
    }
    async fn edit(
        &self,
        _: domain::Snowflake,
        _: domain::Snowflake,
        _: serde_json::Value,
    ) -> AppResult<StoredMessage> {
        Err(AppError::Unavailable("not recorded".into()))
    }
    async fn delete(&self, _: domain::Snowflake, _: domain::Snowflake) -> AppResult<()> {
        Err(AppError::Unavailable("not recorded".into()))
    }
    async fn get(
        &self,
        channel_id: domain::Snowflake,
        message_id: domain::Snowflake,
    ) -> AppResult<Option<StoredMessage>> {
        let stored = self.stored.lock().unwrap();
        Ok(stored
            .get(&(channel_id, message_id))
            .map(|body| StoredMessage {
                id: message_id,
                body: body.clone(),
            }))
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
    async fn get(
        &self,
        _: domain::Snowflake,
        _: domain::Snowflake,
    ) -> AppResult<Option<StoredMessage>> {
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
    ports_with_messages(pool, public_pem, private_pem, Arc::new(NoMessages))
}

/// As [`ports`], with the given `messages` service.
pub fn ports_with_messages(
    pool: &PgPool,
    public_pem: &[u8],
    private_pem: &[u8],
    messages: Arc<dyn MessageService>,
) -> Ports {
    let store = Arc::new(PgStore::new(pool.clone()));
    Ports {
        users: store.clone(),
        sessions: store.clone(),
        account_tokens: store.clone(),
        devices: store.clone(),
        push_tokens: store.clone(),
        relationships: store.clone(),
        reports: store.clone(),
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
        passwords: Arc::new(Argon2Passwords::default()),
        tokens: Arc::new(Ed25519Tokens::from_pem(private_pem, public_pem).expect("jwt keys")),
        clock: Arc::new(SystemClock),
        ids: Arc::new(SnowflakeIds::new(1).expect("worker id")),
        secrets: Arc::new(OsSecrets),
        totp: Arc::new(Rfc6238),
        mailer: Arc::new(DiscardMail),
        limiter: Arc::new(AllowAll),
        presence: Arc::new(NoPresence),
        assets: Arc::new(MemoryAssets::default()),
        messages,
        settings: Settings::default(),
    }
}

/// Presence directory that reports nobody online.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoPresence;

#[async_trait]
impl application::ports::PresenceDirectory for NoPresence {
    async fn online_count(&self, _user_ids: &[domain::Snowflake]) -> AppResult<u64> {
        Ok(0)
    }

    async fn live_ids(&self, _user_ids: &[domain::Snowflake]) -> AppResult<Vec<domain::Snowflake>> {
        Ok(Vec::new())
    }
}

/// Rate limiter that never throttles, for tests that are not about throttling.
#[derive(Debug, Default, Clone, Copy)]
pub struct AllowAll;

#[async_trait]
impl application::ports::RateLimiter for AllowAll {
    async fn hit(
        &self,
        _key: &str,
        limit: u32,
        window: Duration,
    ) -> AppResult<application::ports::RateLimitDecision> {
        Ok(application::ports::RateLimitDecision {
            allowed: true,
            limit,
            remaining: limit,
            reset_after: window,
            retry_after: Duration::ZERO,
        })
    }
}

/// Mailer that drops everything.
pub struct DiscardMail;

#[async_trait]
impl application::ports::Mailer for DiscardMail {
    async fn send(&self, _mail: &application::ports::Mail) -> AppResult<()> {
        Ok(())
    }
}

/// Keeps every mail so tests can read the tokens out of them.
#[derive(Default)]
pub struct RecordingMail {
    pub sent: std::sync::Mutex<Vec<application::ports::Mail>>,
}

impl RecordingMail {
    /// The token in the latest mail to `to`: the text after `paste this code into Voics: `.
    pub fn latest_token(&self, to: &str) -> Option<String> {
        let sent = self.sent.lock().unwrap();
        let mail = sent.iter().rev().find(|mail| mail.to == to)?;
        let marker = "paste this code into Voics: ";
        let start = mail.body.find(marker)? + marker.len();
        Some(mail.body[start..].lines().next()?.trim().to_owned())
    }

    pub fn count_to(&self, to: &str) -> usize {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.to == to)
            .count()
    }
}

#[async_trait]
impl application::ports::Mailer for RecordingMail {
    async fn send(&self, mail: &application::ports::Mail) -> AppResult<()> {
        self.sent.lock().unwrap().push(mail.clone());
        Ok(())
    }
}

/// Allows `budget` hits per key in total, then denies (no refill): enough to test lockouts.
pub struct BudgetLimiter {
    pub budget: u32,
    hits: std::sync::Mutex<std::collections::HashMap<String, u32>>,
}

impl BudgetLimiter {
    pub fn new(budget: u32) -> Self {
        Self {
            budget,
            hits: Default::default(),
        }
    }
}

#[async_trait]
impl application::ports::RateLimiter for BudgetLimiter {
    async fn hit(
        &self,
        key: &str,
        limit: u32,
        window: Duration,
    ) -> AppResult<application::ports::RateLimitDecision> {
        let mut hits = self.hits.lock().unwrap();
        let used = hits.entry(key.to_owned()).or_default();
        *used += 1;
        Ok(application::ports::RateLimitDecision {
            allowed: *used <= self.budget,
            limit,
            remaining: 0,
            reset_after: window,
            retry_after: Duration::from_secs(60),
        })
    }
}
