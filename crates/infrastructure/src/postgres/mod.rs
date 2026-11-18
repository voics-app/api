//! sqlx Postgres repositories and the transactional unit of work.

mod account_tokens;
mod applications;
mod audit;
mod audit_policy;
mod channels;
mod convert;
mod devices;
mod emojis;
mod error;
mod guilds;
mod interactions;
mod invites;
mod notifications;
pub mod outbox;
mod push_tokens;
mod relationships;
mod reports;
mod sessions;
mod unit_of_work;
mod users;
mod vanity;
mod webhooks;

use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

pub use unit_of_work::PgUnitOfWork;

/// Opens the connection pool.
pub async fn connect(database_url: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(10))
        .connect(database_url)
        .await
}

/// Applies the embedded migrations from `api/migrations`.
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("../../migrations").run(pool).await
}

/// Postgres-backed read repositories. Writes go through [`PgUnitOfWork`].
#[derive(Clone)]
pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}
