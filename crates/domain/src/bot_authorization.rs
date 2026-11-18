//! Bot installation identity and explicitly recorded personal consent.
use crate::{permissions::Permissions, validation::FieldError, Snowflake};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotInstallation {
    /// Private database identity; never exposed as an authorization ID.
    pub generation: i64,
    pub guild_id: Snowflake,
    pub application_id: Snowflake,
    pub bot_user_id: Snowflake,
    pub installed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotAuthorization {
    pub id: Snowflake,
    pub user_id: Snowflake,
    pub installation: BotInstallation,
    pub scopes: Vec<String>,
    pub permissions: Permissions,
    pub created_at: DateTime<Utc>,
    pub authorized_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotConsent {
    pub id: Snowflake,
    pub user_id: Snowflake,
    pub guild_id: Snowflake,
    pub application_id: Snowflake,
    pub scopes: Vec<String>,
    pub permissions: Permissions,
    pub authorized_at: DateTime<Utc>,
}

pub fn supported_scopes(scope: &str) -> Result<Vec<String>, FieldError> {
    let scopes: Vec<_> = scope.split_whitespace().collect();
    if !scopes.contains(&"bot")
        || scopes
            .iter()
            .any(|s| !matches!(*s, "bot" | "applications.commands"))
    {
        return Err(FieldError {
            field: "scope".into(),
            code: "OAUTH2_SCOPE_INVALID",
            message: "Supported scopes are bot and optionally applications.commands.".into(),
        });
    }
    let mut normalized = vec!["bot".into()];
    if scopes.contains(&"applications.commands") {
        normalized.push("applications.commands".into());
    }
    Ok(normalized)
}

pub fn requested_permissions(raw: &str) -> Result<Permissions, FieldError> {
    let bits = raw
        .parse::<u64>()
        .ok()
        .filter(|_| !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()));
    match bits {
        Some(bits) if bits & !Permissions::ALL.bits() == 0 => {
            Ok(Permissions::from_bits_truncate(bits))
        }
        _ => Err(FieldError {
            field: "permissions".into(),
            code: "PERMISSIONS_INVALID",
            message: "Use a decimal string containing only supported permission bits.".into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn consent_supports_only_bot_install_scopes_and_known_permissions() {
        assert_eq!(
            supported_scopes("applications.commands bot bot").unwrap(),
            vec!["bot", "applications.commands"]
        );
        for scope in ["", "identify", "bot guilds", "applications.commands"] {
            assert!(supported_scopes(scope).is_err());
        }
        for bits in ["-1", "+8", "8.5", "9223372036854775808", "140737488355328"] {
            assert!(requested_permissions(bits).is_err());
        }
        assert_eq!(
            requested_permissions("8").unwrap(),
            Permissions::ADMINISTRATOR
        );
    }
}
