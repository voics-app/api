//! Bot installation identity and explicitly recorded personal consent.
use crate::{permissions::Permissions, validation::FieldError, Snowflake};
use chrono::{DateTime, Utc};

/// The installed/absent state shown by a consent preview, including removal history.
/// Clients echo the wire value unchanged; it is not an authorization ID or credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotInstallationRevision {
    Absent(Option<i64>),
    Installed(i64),
}

impl BotInstallationRevision {
    pub fn generation(self) -> Option<i64> {
        match self {
            Self::Absent(_) => None,
            Self::Installed(generation) => Some(generation),
        }
    }

    pub fn last_generation(self) -> Option<i64> {
        match self {
            Self::Absent(last) => last,
            Self::Installed(generation) => Some(generation),
        }
    }

    pub fn to_wire(self) -> String {
        match self {
            Self::Absent(last) => format!("absent:{}", last.unwrap_or(0)),
            Self::Installed(generation) => format!("installed:{generation}"),
        }
    }

    pub fn parse(raw: &str) -> Result<Self, FieldError> {
        let invalid = || FieldError {
            field: "installation_revision".into(),
            code: "INSTALLATION_REVISION_INVALID",
            message: "Use the installation revision from a fresh authorization preview.".into(),
        };
        let (state, value) = raw.split_once(':').ok_or_else(invalid)?;
        if value.is_empty()
            || !value.bytes().all(|byte| byte.is_ascii_digit())
            || (value.len() > 1 && value.starts_with('0'))
        {
            return Err(invalid());
        }
        let generation = value.parse::<i64>().map_err(|_| invalid())?;
        match (state, generation) {
            ("absent", 0) => Ok(Self::Absent(None)),
            ("absent", generation) if generation > 0 => Ok(Self::Absent(Some(generation))),
            ("installed", generation) if generation > 0 => Ok(Self::Installed(generation)),
            _ => Err(invalid()),
        }
    }
}

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
    fn installation_revisions_preserve_presence_and_removal_history() {
        for revision in [
            BotInstallationRevision::Absent(None),
            BotInstallationRevision::Absent(Some(42)),
            BotInstallationRevision::Installed(42),
        ] {
            assert_eq!(
                BotInstallationRevision::parse(&revision.to_wire()),
                Ok(revision)
            );
        }
        assert_ne!(
            BotInstallationRevision::Absent(None),
            BotInstallationRevision::Absent(Some(42))
        );
        for invalid in [
            "",
            "absent",
            "absent:-1",
            "absent:01",
            "installed:0",
            "installed:+1",
            "installed:9223372036854775808",
            "other:1",
        ] {
            assert!(BotInstallationRevision::parse(invalid).is_err());
        }
    }

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
