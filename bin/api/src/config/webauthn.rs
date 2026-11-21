//! A single explicit HTTPS origin owns ceremonies. CORS and native file origins
//! cannot alter that trust boundary.

use super::{invalid, required, ConfigError, Environment};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebauthnConfig {
    pub rp_id: String,
    pub origin: Url,
}

pub fn parse(
    get: &impl Fn(&str) -> Option<String>,
    environment: Environment,
) -> Result<Option<WebauthnConfig>, ConfigError> {
    let present = |key| get(key).is_some_and(|value| !value.trim().is_empty());
    if !present("WEBAUTHN_RP_ID") && !present("WEBAUTHN_RP_ORIGIN") {
        return Ok(None);
    }
    let rp_id = required(get, "WEBAUTHN_RP_ID")?;
    let origin = Url::parse(&required(get, "WEBAUTHN_RP_ORIGIN")?)
        .map_err(|_| invalid("WEBAUTHN_RP_ORIGIN", "expected an absolute origin URL"))?;
    let host = origin
        .host_str()
        .ok_or_else(|| invalid("WEBAUTHN_RP_ORIGIN", "expected a hostname"))?;
    let local = environment == Environment::Local && host == "localhost";
    if origin.scheme() != "https" && !(local && origin.scheme() == "http") {
        return Err(invalid(
            "WEBAUTHN_RP_ORIGIN",
            "HTTPS is required (HTTP localhost is permitted only in local development)",
        ));
    }
    if origin.path() != "/"
        || origin.query().is_some()
        || origin.fragment().is_some()
        || !origin.username().is_empty()
        || origin.password().is_some()
    {
        return Err(invalid(
            "WEBAUTHN_RP_ORIGIN",
            "use only scheme, hostname and optional port",
        ));
    }
    // WebauthnBuilder also validates the RP/domain relationship; keep malformed
    // deployment configuration out of startup before building any adapters.
    if rp_id != rp_id.to_ascii_lowercase()
        || rp_id.ends_with('.')
        || rp_id.starts_with('.')
        || !rp_id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'.'
        })
        || !(host == rp_id || host.ends_with(&format!(".{rp_id}")))
    {
        return Err(invalid(
            "WEBAUTHN_RP_ID",
            "expected the origin hostname or its parent domain",
        ));
    }
    Ok(Some(WebauthnConfig { rp_id, origin }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config(
        id: Option<&str>,
        origin: Option<&str>,
        environment: Environment,
    ) -> Result<Option<WebauthnConfig>, ConfigError> {
        parse(
            &|key| match key {
                "WEBAUTHN_RP_ID" => id.map(str::to_owned),
                "WEBAUTHN_RP_ORIGIN" => origin.map(str::to_owned),
                _ => None,
            },
            environment,
        )
    }

    #[test]
    fn configuration_is_optional_but_must_be_complete() {
        assert_eq!(config(None, None, Environment::Production).unwrap(), None);
        assert!(config(Some("example.com"), None, Environment::Production).is_err());
        assert!(config(None, Some("https://example.com"), Environment::Production).is_err());
        assert!(config(
            Some("example.com"),
            Some("https://auth.example.com"),
            Environment::Production
        )
        .is_ok());
    }

    #[test]
    fn only_exact_https_origins_or_local_http_are_accepted() {
        for origin in [
            "null",
            "file:///app.html",
            "https://evil-example.com",
            "http://example.com",
            "https://example.com/path",
            "https://example.com?x=1",
            "https://example.com/#x",
            "https://user:pass@example.com",
        ] {
            assert!(
                config(Some("example.com"), Some(origin), Environment::Production).is_err(),
                "{origin}"
            );
        }
        assert!(config(
            Some("localhost"),
            Some("http://localhost:5173"),
            Environment::Local
        )
        .is_ok());
        assert!(config(
            Some("localhost"),
            Some("http://localhost:5173"),
            Environment::Production
        )
        .is_err());
        assert!(config(
            Some("example.com:443"),
            Some("https://example.com"),
            Environment::Production
        )
        .is_err());
    }
}
