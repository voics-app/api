//! Composition root: reads configuration, wires the production adapters into
//! the application ports, starts the outbox relay and serves HTTP.

mod config;
mod telemetry;

use std::error::Error;
use std::process::ExitCode;
use std::sync::Arc;

use tokio::net::TcpListener;

use application::use_cases::account::purge_deleted;
use application::use_cases::devices::prune_stale;
use application::{Ports, Settings, VoiceRegion};
use config::{Config, Environment, MailTransport};
use infrastructure::events::direct::KafkaEvents;
use infrastructure::events::kafka::KafkaPublisher;
use infrastructure::events::relay::OutboxRelay;
use infrastructure::mail::{LogMailer, SmtpMailer, SmtpSettings, SmtpTls};
use infrastructure::messages_client::HttpMessages;
use infrastructure::postgres::{connect, migrate, PgStore, PgUnitOfWork};
use infrastructure::presence::ValkeyPresence;
use infrastructure::ratelimit::ValkeyRateLimiter;
use infrastructure::security::jwt::Ed25519Tokens;
use infrastructure::security::password::Argon2Passwords;
use infrastructure::security::secrets::OsSecrets;
use infrastructure::security::totp::Rfc6238;
use infrastructure::storage::MinioAssets;
use infrastructure::system::clock::SystemClock;
use infrastructure::system::ids::SnowflakeIds;
use interface::state::{AppState, RatePolicy};
use telemetry::Telemetry;

const DATABASE_MAX_CONNECTIONS: u32 = 16;

#[tokio::main]
async fn main() -> ExitCode {
    let config = match Config::from_lookup(|name| std::env::var(name).ok()) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("configuration error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let telemetry = match Telemetry::install(&config.log_level, config.otlp_endpoint.as_ref()) {
        Ok(telemetry) => telemetry,
        Err(error) => {
            eprintln!("telemetry error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let outcome = run(config).await;
    telemetry.shutdown();
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "api stopped with an error");
            ExitCode::FAILURE
        }
    }
}

async fn run(config: Config) -> Result<(), Box<dyn Error>> {
    let pool = connect(&config.database_url, DATABASE_MAX_CONNECTIONS).await?;
    migrate(&pool).await?;

    let region = config.default_voice_region.clone();
    let publisher = Arc::new(KafkaPublisher::connect(config.kafka_brokers.clone()).await?);
    let store = Arc::new(PgStore::new(pool.clone()));
    let rate_limiter = Arc::new(ValkeyRateLimiter::connect(&config.valkey_url).await?);
    let mailer: Arc<dyn application::ports::Mailer> = match &config.mail {
        MailTransport::Log => Arc::new(LogMailer {
            include_body: config.environment == Environment::Local,
        }),
        MailTransport::Smtp(smtp) => Arc::new(SmtpMailer::new(&SmtpSettings {
            host: smtp.host.clone(),
            port: smtp.port,
            tls: match smtp.tls.as_str() {
                "tls" => SmtpTls::Implicit,
                "none" => SmtpTls::None,
                _ => SmtpTls::StartTls,
            },
            username: smtp.username.clone(),
            password: smtp.password.clone(),
            from: smtp.from.clone(),
        })?),
    };
    let ports = Ports {
        account_tokens: store.clone(),
        mailer,
        limiter: rate_limiter.clone(),
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
        unit_of_work: Arc::new(PgUnitOfWork::new(pool.clone(), region.clone())),
        events: Arc::new(KafkaEvents::new(publisher.clone(), region.clone())),
        passwords: Arc::new(Argon2Passwords::new(
            config.password_hash_concurrency,
            config.password_hash_queue_timeout,
        )),
        tokens: Arc::new(Ed25519Tokens::from_pem(
            config.jwt_private_pem.as_bytes(),
            config.jwt_public_pem.as_bytes(),
        )?),
        clock: Arc::new(SystemClock),
        presence: Arc::new(ValkeyPresence::connect(&config.valkey_url).await?),
        ids: Arc::new(SnowflakeIds::new(config.worker_id)?),
        secrets: Arc::new(OsSecrets),
        totp: Arc::new(Rfc6238),
        assets: Arc::new(MinioAssets::new(
            config.object_store.endpoint.clone(),
            config.object_store.assets_bucket.clone(),
            config.object_store.access_key.clone(),
            config.object_store.secret_key.clone(),
        )?),
        messages: Arc::new(HttpMessages::new(
            config.messages_internal_url.clone(),
            config.internal_token.clone(),
        )),
        settings: Settings {
            voice_regions: vec![VoiceRegion {
                id: region.clone(),
                name: region.clone(),
            }],
            default_voice_region: region,
            public_api_base: config
                .public_api_url
                .as_str()
                .trim_end_matches('/')
                .to_owned(),
            device_max_idle: chrono::Duration::days(i64::from(config.device_max_idle_days)),
            brand_name: config.brand_name.clone(),
            app_link_base: config.app_link_base.clone(),
            account_deletion_retention: chrono::Duration::days(i64::from(
                config.account_deletion_retention_days,
            )),
            discovery_min_members: config.discovery_min_members,
            discovery_min_guild_age: chrono::Duration::days(i64::from(
                config.discovery_min_guild_age_days,
            )),
            group_dm_max_participants: config.group_dm_max_participants,
            ..Settings::default()
        },
    };
    let prune_ports = ports.clone();
    let purge_ports = ports.clone();
    let purge_every = std::time::Duration::from_secs(config.account_purge_interval_secs);
    let prune_every = std::time::Duration::from_secs(config.device_prune_interval_secs);

    tokio::spawn(OutboxRelay::new(pool.clone(), publisher.clone()).run());
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(prune_every);
        loop {
            tick.tick().await;
            match prune_stale::execute(&prune_ports).await {
                Ok(0) => {}
                Ok(count) => tracing::info!(count, "pruned stale devices"),
                Err(error) => tracing::warn!(%error, "stale device prune failed"),
            }
        }
    });

    tokio::spawn(async move {
        let mut tick = tokio::time::interval(purge_every);
        loop {
            tick.tick().await;
            match purge_deleted::execute(&purge_ports).await {
                Ok(0) => {}
                Ok(count) => tracing::info!(count, "purged deleted accounts"),
                Err(error) => tracing::warn!(%error, "account purge failed"),
            }
        }
    });

    let state = AppState {
        ports,
        internal_token: Arc::from(config.internal_token.as_str()),
        rate_limiter,
        rate_policy: RatePolicy::default(),
    };
    let listener = TcpListener::bind(config.bind_addr).await?;
    tracing::info!(addr = %config.bind_addr, "api listening");
    let metrics = telemetry::prometheus()?;
    let cors = interface::cors::layer(&config.cors_allowed_origins)?;
    let app = interface::router(state)
        .route(
            "/metrics",
            axum::routing::get(move || {
                let metrics = metrics.clone();
                async move { metrics.render() }
            }),
        )
        .layer(cors);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}
