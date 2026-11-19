//! Startup configuration from environment variables (P2). Validation happens
//! once, here; nothing downstream reads the environment.

use std::net::SocketAddr;
use std::time::Duration;

use url::Url;

mod webauthn;
pub use webauthn::WebauthnConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Local,
    Production,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectStore {
    pub endpoint: Url,
    pub access_key: String,
    pub secret_key: String,
    pub assets_bucket: String,
}

/// How account emails leave the api.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailTransport {
    /// Logged, not sent (local development, or production before SMTP exists).
    Log,
    Smtp(SmtpConfig),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    /// `starttls`, `tls` or `none`.
    pub tls: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub environment: Environment,
    pub log_level: String,
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub valkey_url: String,
    pub kafka_brokers: Vec<String>,
    pub internal_token: String,
    pub jwt_private_pem: String,
    pub jwt_public_pem: String,
    pub worker_id: u16,
    pub public_api_url: Url,
    pub public_gateway_url: Url,
    pub gateway_internal_url: Option<Url>,
    pub cdn_public_url: Url,
    pub messages_internal_url: Url,
    pub default_voice_region: String,
    pub object_store: ObjectStore,
    pub otlp_endpoint: Option<Url>,
    /// Origins allowed by CORS; `null` is the origin of the packaged `file://` client.
    pub cors_allowed_origins: Vec<String>,
    /// Devices idle for more than this many days leave E2EE device listings and are pruned.
    pub device_max_idle_days: u32,
    /// Seconds between two stale-device prune runs.
    pub device_prune_interval_secs: u64,
    pub mail: MailTransport,
    pub webauthn: Option<WebauthnConfig>,
    /// Product name shown in emails and as the authenticator-app issuer.
    pub brand_name: String,
    /// Prefix of the links in account emails.
    pub app_link_base: String,
    /// Days a deleted account stays recoverable by logging in.
    pub account_deletion_retention_days: u32,
    /// Seconds between two runs that anonymize accounts past retention.
    pub account_purge_interval_secs: u64,
    /// Members a guild needs before it can be listed in Discover.
    pub discovery_min_members: u64,
    /// Days a guild must exist before it can be listed in Discover.
    pub discovery_min_guild_age_days: u32,
    /// Most members a group DM can have, the creator included.
    pub group_dm_max_participants: usize,
    /// Password hashes (argon2) allowed to run at once; each costs tens of MB.
    pub password_hash_concurrency: usize,
    /// How long a password operation waits for a free hash slot before a 429.
    pub password_hash_queue_timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("missing required environment variable {0}")]
    Missing(&'static str),
    #[error("{name} is invalid: {reason}")]
    Invalid { name: &'static str, reason: String },
}

const WORKER_ID_MAX: u16 = 63;
const DEFAULT_CORS_ALLOWED_ORIGINS: &str = "null,http://localhost:5173";
const DEFAULT_DEVICE_MAX_IDLE_DAYS: &str = "30";
const DEFAULT_DEVICE_PRUNE_INTERVAL_SECS: &str = "3600";
const DEFAULT_ACCOUNT_DELETION_DAYS: &str = "14";
const DEFAULT_ACCOUNT_PURGE_INTERVAL_SECS: &str = "3600";
const DEFAULT_DISCOVERY_MIN_MEMBERS: &str = "5";
const DEFAULT_DISCOVERY_MIN_AGE_DAYS: &str = "7";
const DEFAULT_GROUP_DM_MAX_PARTICIPANTS: &str = "25";
const DEFAULT_PASSWORD_HASH_CONCURRENCY: &str = "2";
const DEFAULT_PASSWORD_HASH_QUEUE_TIMEOUT_MS: &str = "5000";
const DEFAULT_SMTP_PORT: &str = "587";

fn mail_transport(get: &impl Fn(&str) -> Option<String>) -> Result<MailTransport, ConfigError> {
    match optional(get, "MAIL_TRANSPORT", "log").as_str() {
        "log" => Ok(MailTransport::Log),
        "smtp" => {
            let tls = optional(get, "SMTP_TLS", "starttls");
            if !["starttls", "tls", "none"].contains(&tls.as_str()) {
                return Err(invalid("SMTP_TLS", "expected starttls, tls or none"));
            }
            let present = |name: &str| {
                get(name)
                    .map(|v| v.trim().to_owned())
                    .filter(|v| !v.is_empty())
            };
            Ok(MailTransport::Smtp(SmtpConfig {
                host: required(get, "SMTP_HOST")?,
                port: positive(get, "SMTP_PORT", DEFAULT_SMTP_PORT)?,
                tls,
                username: present("SMTP_USERNAME"),
                password: present("SMTP_PASSWORD"),
                from: required(get, "MAIL_FROM")?,
            }))
        }
        other => Err(invalid(
            "MAIL_TRANSPORT",
            format!("expected log or smtp, got {other}"),
        )),
    }
}

/// A strictly positive integer variable with a default.
fn positive<T: std::str::FromStr + PartialOrd + From<u8>>(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: &str,
) -> Result<T, ConfigError> {
    optional(get, name, default)
        .parse::<T>()
        .ok()
        .filter(|value| *value >= T::from(1))
        .ok_or_else(|| invalid(name, "expected a positive integer"))
}

/// A group DM needs room for its creator and one other member.
fn group_dm_max_participants(get: &impl Fn(&str) -> Option<String>) -> Result<usize, ConfigError> {
    const NAME: &str = "GROUP_DM_MAX_PARTICIPANTS";
    let value: usize = positive(get, NAME, DEFAULT_GROUP_DM_MAX_PARTICIPANTS)?;
    if value < 2 {
        return Err(invalid(NAME, "expected at least 2"));
    }
    Ok(value)
}

/// Splits a comma-separated list, trimming blanks and dropping empty entries.
fn split_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

fn invalid(name: &'static str, reason: impl Into<String>) -> ConfigError {
    ConfigError::Invalid {
        name,
        reason: reason.into(),
    }
}

fn required(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
) -> Result<String, ConfigError> {
    get(name)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or(ConfigError::Missing(name))
}

fn optional(get: &impl Fn(&str) -> Option<String>, name: &str, default: &str) -> String {
    get(name)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn url(get: &impl Fn(&str) -> Option<String>, name: &'static str) -> Result<Url, ConfigError> {
    let raw = required(get, name)?;
    Url::parse(&raw).map_err(|e| invalid(name, e.to_string()))
}

fn pem(get: &impl Fn(&str) -> Option<String>, name: &'static str) -> Result<String, ConfigError> {
    let raw = required(get, name)?;
    if raw.contains("-----BEGIN") {
        Ok(raw.replace("\\n", "\n"))
    } else {
        Err(invalid(name, "expected a PEM-encoded Ed25519 key"))
    }
}

impl Config {
    /// Reads and validates the configuration; `get` is `std::env::var` in production.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let environment = match optional(&get, "DISCORDY_ENV", "local").as_str() {
            "local" => Environment::Local,
            "production" => Environment::Production,
            other => {
                return Err(invalid(
                    "DISCORDY_ENV",
                    format!("expected local or production, got {other}"),
                ))
            }
        };
        let worker_raw = required(&get, "SNOWFLAKE_WORKER_ID")?;
        let worker_id: u16 = worker_raw
            .parse()
            .ok()
            .filter(|id| *id <= WORKER_ID_MAX)
            .ok_or_else(|| {
                invalid(
                    "SNOWFLAKE_WORKER_ID",
                    "expected a preferred API worker between 0 and 63",
                )
            })?;
        let bind_addr = optional(&get, "API_BIND_ADDR", "0.0.0.0:8080")
            .parse()
            .map_err(|_| invalid("API_BIND_ADDR", "expected host:port"))?;
        let kafka_brokers = split_list(&required(&get, "KAFKA_BROKERS")?);
        let otlp_endpoint =
            match get("OTEL_EXPORTER_OTLP_ENDPOINT").filter(|v| !v.trim().is_empty()) {
                Some(raw) => Some(
                    Url::parse(raw.trim())
                        .map_err(|e| invalid("OTEL_EXPORTER_OTLP_ENDPOINT", e.to_string()))?,
                ),
                None => None,
            };
        let gateway_internal_url =
            match get("GATEWAY_INTERNAL_URL").filter(|v| !v.trim().is_empty()) {
                Some(raw) => {
                    let parsed = Url::parse(raw.trim())
                        .map_err(|e| invalid("GATEWAY_INTERNAL_URL", e.to_string()))?;
                    if !matches!(parsed.scheme(), "http" | "https")
                        || parsed.host_str().is_none()
                        || !parsed.username().is_empty()
                        || parsed.password().is_some()
                    {
                        return Err(invalid(
                            "GATEWAY_INTERNAL_URL",
                            "expected an HTTP(S) service URL without credentials",
                        ));
                    }
                    Some(parsed)
                }
                None => None,
            };

        let cors_allowed_origins = split_list(&optional(
            &get,
            "CORS_ALLOWED_ORIGINS",
            DEFAULT_CORS_ALLOWED_ORIGINS,
        ));

        Ok(Self {
            environment,
            log_level: optional(&get, "LOG_LEVEL", "info"),
            bind_addr,
            database_url: required(&get, "DATABASE_URL")?,
            valkey_url: required(&get, "VALKEY_URL")?,
            kafka_brokers,
            internal_token: required(&get, "INTERNAL_SERVICE_TOKEN")?,
            jwt_private_pem: pem(&get, "JWT_PRIVATE_KEY")?,
            jwt_public_pem: pem(&get, "JWT_PUBLIC_KEY")?,
            worker_id,
            public_api_url: url(&get, "PUBLIC_API_URL")?,
            public_gateway_url: url(&get, "PUBLIC_GATEWAY_URL")?,
            gateway_internal_url,
            cdn_public_url: url(&get, "CDN_PUBLIC_URL")?,
            messages_internal_url: url(&get, "MESSAGES_INTERNAL_URL")?,
            default_voice_region: optional(&get, "DEFAULT_VOICE_REGION", "nl-home"),
            object_store: ObjectStore {
                endpoint: url(&get, "MINIO_ENDPOINT")?,
                access_key: required(&get, "MINIO_ACCESS_KEY")?,
                secret_key: required(&get, "MINIO_SECRET_KEY")?,
                assets_bucket: optional(&get, "MINIO_BUCKET_ASSETS", "discordy-assets"),
            },
            otlp_endpoint,
            cors_allowed_origins,
            device_max_idle_days: positive(
                &get,
                "DEVICE_MAX_IDLE_DAYS",
                DEFAULT_DEVICE_MAX_IDLE_DAYS,
            )?,
            device_prune_interval_secs: positive(
                &get,
                "DEVICE_PRUNE_INTERVAL_SECS",
                DEFAULT_DEVICE_PRUNE_INTERVAL_SECS,
            )?,
            mail: mail_transport(&get)?,
            webauthn: webauthn::parse(&get, environment)?,
            brand_name: optional(&get, "BRAND_NAME", application::context::DEFAULT_BRAND_NAME),
            app_link_base: optional(&get, "APP_LINK_BASE", "voics://"),
            account_deletion_retention_days: positive(
                &get,
                "ACCOUNT_DELETION_RETENTION_DAYS",
                DEFAULT_ACCOUNT_DELETION_DAYS,
            )?,
            discovery_min_members: positive(
                &get,
                "DISCOVERY_MIN_MEMBERS",
                DEFAULT_DISCOVERY_MIN_MEMBERS,
            )?,
            discovery_min_guild_age_days: positive(
                &get,
                "DISCOVERY_MIN_GUILD_AGE_DAYS",
                DEFAULT_DISCOVERY_MIN_AGE_DAYS,
            )?,
            group_dm_max_participants: group_dm_max_participants(&get)?,
            account_purge_interval_secs: positive(
                &get,
                "ACCOUNT_PURGE_INTERVAL_SECS",
                DEFAULT_ACCOUNT_PURGE_INTERVAL_SECS,
            )?,
            password_hash_concurrency: positive(
                &get,
                "PASSWORD_HASH_CONCURRENCY",
                DEFAULT_PASSWORD_HASH_CONCURRENCY,
            )?,
            password_hash_queue_timeout: Duration::from_millis(positive(
                &get,
                "PASSWORD_HASH_QUEUE_TIMEOUT_MS",
                DEFAULT_PASSWORD_HASH_QUEUE_TIMEOUT_MS,
            )?),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    const PEM: &str = "-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----";

    fn complete() -> HashMap<&'static str, String> {
        HashMap::from([
            ("SNOWFLAKE_WORKER_ID", "7".to_owned()),
            (
                "DATABASE_URL",
                "postgres://u:p@localhost/discordy".to_owned(),
            ),
            ("VALKEY_URL", "redis://localhost:6379".to_owned()),
            ("KAFKA_BROKERS", "localhost:19092".to_owned()),
            ("INTERNAL_SERVICE_TOKEN", "dev-internal".to_owned()),
            ("JWT_PRIVATE_KEY", PEM.to_owned()),
            ("JWT_PUBLIC_KEY", PEM.to_owned()),
            ("PUBLIC_API_URL", "http://localhost:8080".to_owned()),
            ("PUBLIC_GATEWAY_URL", "ws://localhost:4000".to_owned()),
            ("CDN_PUBLIC_URL", "http://localhost:9000".to_owned()),
            ("MESSAGES_INTERNAL_URL", "http://localhost:8082".to_owned()),
            ("MINIO_ENDPOINT", "http://localhost:9000".to_owned()),
            ("MINIO_ACCESS_KEY", "minio".to_owned()),
            ("MINIO_SECRET_KEY", "minio-secret".to_owned()),
        ])
    }

    fn lookup(values: HashMap<&'static str, String>) -> impl Fn(&str) -> Option<String> {
        move |name| values.get(name).cloned()
    }

    #[test]
    fn complete_environment_loads_with_defaults() {
        let config = Config::from_lookup(lookup(complete())).expect("valid");
        assert_eq!(config.environment, Environment::Local);
        assert_eq!(config.worker_id, 7);
        assert_eq!(config.bind_addr, "0.0.0.0:8080".parse().unwrap());
        assert_eq!(config.object_store.assets_bucket, "discordy-assets");
        assert_eq!(config.default_voice_region, "nl-home");
        assert_eq!(config.kafka_brokers, vec!["localhost:19092".to_owned()]);
    }

    #[test]
    fn missing_required_variables_are_named() {
        let mut values = complete();
        values.remove("DATABASE_URL");
        assert_eq!(
            Config::from_lookup(lookup(values)),
            Err(ConfigError::Missing("DATABASE_URL"))
        );
    }

    #[test]
    fn worker_id_must_fit_the_snowflake_range() {
        let mut values = complete();
        values.insert("SNOWFLAKE_WORKER_ID", "1024".to_owned());
        assert!(matches!(
            Config::from_lookup(lookup(values)),
            Err(ConfigError::Invalid {
                name: "SNOWFLAKE_WORKER_ID",
                ..
            })
        ));
    }

    #[test]
    fn keys_must_be_pem_encoded() {
        let mut values = complete();
        values.insert("JWT_PUBLIC_KEY", "not-a-key".to_owned());
        assert!(matches!(
            Config::from_lookup(lookup(values)),
            Err(ConfigError::Invalid {
                name: "JWT_PUBLIC_KEY",
                ..
            })
        ));
    }

    #[test]
    fn escaped_newlines_in_pem_are_restored() {
        let mut values = complete();
        values.insert(
            "JWT_PRIVATE_KEY",
            "-----BEGIN PRIVATE KEY-----\\nAAAA\\n-----END PRIVATE KEY-----".to_owned(),
        );
        let config = Config::from_lookup(lookup(values)).expect("valid");
        assert!(config.jwt_private_pem.contains("\nAAAA\n"));
    }

    #[test]
    fn cors_origins_default_to_the_packaged_client_and_dev_server() {
        let config = Config::from_lookup(lookup(complete())).expect("valid");
        assert_eq!(
            config.cors_allowed_origins,
            vec!["null".to_owned(), "http://localhost:5173".to_owned()]
        );
    }

    #[test]
    fn cors_origins_are_parsed_from_a_comma_list() {
        let mut values = complete();
        values.insert(
            "CORS_ALLOWED_ORIGINS",
            " https://app.discordy.example , null ,".to_owned(),
        );
        let config = Config::from_lookup(lookup(values)).expect("valid");
        assert_eq!(
            config.cors_allowed_origins,
            vec!["https://app.discordy.example".to_owned(), "null".to_owned()]
        );
    }

    #[test]
    fn device_pruning_defaults_to_thirty_idle_days_checked_hourly() {
        let config = Config::from_lookup(lookup(complete())).expect("valid");
        assert_eq!(config.device_max_idle_days, 30);
        assert_eq!(config.device_prune_interval_secs, 3600);
    }

    #[test]
    fn device_pruning_is_configurable_and_must_be_positive() {
        let mut values = complete();
        values.insert("DEVICE_MAX_IDLE_DAYS", "90".to_owned());
        values.insert("DEVICE_PRUNE_INTERVAL_SECS", "600".to_owned());
        let config = Config::from_lookup(lookup(values)).expect("valid");
        assert_eq!(config.device_max_idle_days, 90);
        assert_eq!(config.device_prune_interval_secs, 600);

        for (name, bad) in [
            ("DEVICE_MAX_IDLE_DAYS", "0"),
            ("DEVICE_MAX_IDLE_DAYS", "soon"),
            ("DEVICE_PRUNE_INTERVAL_SECS", "0"),
        ] {
            let mut values = complete();
            values.insert(name, bad.to_owned());
            assert!(
                matches!(
                    Config::from_lookup(lookup(values)),
                    Err(ConfigError::Invalid { name: n, .. }) if n == name
                ),
                "{name}={bad}"
            );
        }
    }

    #[test]
    fn password_hashing_is_bounded_by_default_and_configurable() {
        let config = Config::from_lookup(lookup(complete())).expect("valid");
        assert_eq!(config.password_hash_concurrency, 2);
        assert_eq!(config.password_hash_queue_timeout, Duration::from_secs(5));

        let mut values = complete();
        values.insert("PASSWORD_HASH_CONCURRENCY", "3".to_owned());
        values.insert("PASSWORD_HASH_QUEUE_TIMEOUT_MS", "1500".to_owned());
        let config = Config::from_lookup(lookup(values)).expect("valid");
        assert_eq!(config.password_hash_concurrency, 3);
        assert_eq!(
            config.password_hash_queue_timeout,
            Duration::from_millis(1500)
        );

        for name in [
            "PASSWORD_HASH_CONCURRENCY",
            "PASSWORD_HASH_QUEUE_TIMEOUT_MS",
        ] {
            let mut values = complete();
            values.insert(name, "0".to_owned());
            assert!(matches!(
                Config::from_lookup(lookup(values)),
                Err(ConfigError::Invalid { name: n, .. }) if n == name
            ));
        }
    }

    #[test]
    fn mail_defaults_to_the_log_transport() {
        let config = Config::from_lookup(lookup(complete())).expect("valid");
        assert_eq!(config.mail, MailTransport::Log);
        assert_eq!(config.account_deletion_retention_days, 14);
        assert_eq!(config.discovery_min_members, 5);
        assert_eq!(config.discovery_min_guild_age_days, 7);
    }

    #[test]
    fn group_dm_cap_defaults_to_25_and_is_configurable() {
        let config = Config::from_lookup(lookup(complete())).expect("valid");
        assert_eq!(config.group_dm_max_participants, 25);

        let mut values = complete();
        values.insert("GROUP_DM_MAX_PARTICIPANTS", "40".to_owned());
        let config = Config::from_lookup(lookup(values)).expect("valid");
        assert_eq!(config.group_dm_max_participants, 40);

        for bad in ["0", "1", "many"] {
            let mut values = complete();
            values.insert("GROUP_DM_MAX_PARTICIPANTS", bad.to_owned());
            assert!(matches!(
                Config::from_lookup(lookup(values)),
                Err(ConfigError::Invalid {
                    name: "GROUP_DM_MAX_PARTICIPANTS",
                    ..
                })
            ));
        }
    }

    #[test]
    fn smtp_transport_needs_host_and_sender() {
        let mut values = complete();
        values.insert("MAIL_TRANSPORT", "smtp".to_owned());
        assert_eq!(
            Config::from_lookup(lookup(values.clone())),
            Err(ConfigError::Missing("SMTP_HOST"))
        );
        values.insert("SMTP_HOST", "smtp.example.com".to_owned());
        values.insert("MAIL_FROM", "Example <no-reply@example.com>".to_owned());
        values.insert("SMTP_PASSWORD", "  ".to_owned());
        let config = Config::from_lookup(lookup(values)).expect("valid");
        let MailTransport::Smtp(smtp) = config.mail else {
            panic!("smtp expected");
        };
        assert_eq!((smtp.port, smtp.tls.as_str()), (587, "starttls"));
        assert_eq!(smtp.password, None);
    }

    #[test]
    fn unknown_mail_transport_is_rejected() {
        let mut values = complete();
        values.insert("MAIL_TRANSPORT", "pigeon".to_owned());
        assert!(matches!(
            Config::from_lookup(lookup(values)),
            Err(ConfigError::Invalid {
                name: "MAIL_TRANSPORT",
                ..
            })
        ));
    }

    #[test]
    fn production_environment_is_recognised() {
        let mut values = complete();
        values.insert("DISCORDY_ENV", "production".to_owned());
        assert_eq!(
            Config::from_lookup(lookup(values)).unwrap().environment,
            Environment::Production
        );
    }
}
