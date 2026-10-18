//! Startup configuration from environment variables (P2). Validation happens
//! once, here; nothing downstream reads the environment.

use std::net::SocketAddr;

use url::Url;

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
    pub cdn_public_url: Url,
    pub messages_internal_url: Url,
    pub default_voice_region: String,
    pub object_store: ObjectStore,
    pub otlp_endpoint: Option<Url>,
    /// Origins allowed by CORS; `null` is the origin of the packaged `file://` client.
    pub cors_allowed_origins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("missing required environment variable {0}")]
    Missing(&'static str),
    #[error("{name} is invalid: {reason}")]
    Invalid { name: &'static str, reason: String },
}

const WORKER_ID_MAX: u16 = 1023;
const DEFAULT_CORS_ALLOWED_ORIGINS: &str = "null,http://localhost:5173";

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
                    "expected an integer between 0 and 1023",
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
    fn production_environment_is_recognised() {
        let mut values = complete();
        values.insert("DISCORDY_ENV", "production".to_owned());
        assert_eq!(
            Config::from_lookup(lookup(values)).unwrap().environment,
            Environment::Production
        );
    }
}
