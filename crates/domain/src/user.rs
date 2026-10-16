use chrono::{DateTime, Utc};

use crate::delivery_mode::DeliveryMode;
use crate::snowflake::Snowflake;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: Snowflake,
    pub username: String,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
    pub bot: bool,
    pub system: bool,
    pub email: Option<String>,
    pub verified: bool,
    pub mfa_enabled: bool,
    pub flags: u64,
    pub platform_admin: bool,
    pub suspension: Option<Suspension>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suspension {
    pub reason: String,
    /// `None` means indefinitely.
    pub until: Option<DateTime<Utc>>,
}

impl User {
    pub fn is_suspended(&self, now: DateTime<Utc>) -> bool {
        match &self.suspension {
            Some(Suspension { until: Some(until), .. }) => *until > now,
            Some(Suspension { until: None, .. }) => true,
            None => false,
        }
    }

    pub fn display_name(&self) -> &str {
        self.global_name.as_deref().unwrap_or(&self.username)
    }
}

/// Secrets that never leave the auth use cases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub user_id: Snowflake,
    pub password_hash: String,
    pub totp_secret: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PresenceStatus {
    #[default]
    Online,
    Idle,
    Dnd,
    Invisible,
}

impl PresenceStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            PresenceStatus::Online => "online",
            PresenceStatus::Idle => "idle",
            PresenceStatus::Dnd => "dnd",
            PresenceStatus::Invisible => "invisible",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "online" => Some(PresenceStatus::Online),
            "idle" => Some(PresenceStatus::Idle),
            "dnd" => Some(PresenceStatus::Dnd),
            "invisible" => Some(PresenceStatus::Invisible),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Oled,
    Light,
}

impl Theme {
    pub const fn as_str(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Oled => "oled",
            Theme::Light => "light",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "dark" => Some(Theme::Dark),
            "oled" => Some(Theme::Oled),
            "light" => Some(Theme::Light),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomStatus {
    pub text: Option<String>,
    pub emoji_name: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSettings {
    pub user_id: Snowflake,
    pub default_delivery_mode: DeliveryMode,
    pub theme: Theme,
    pub locale: String,
    pub status: PresenceStatus,
    pub custom_status: Option<CustomStatus>,
    pub developer_mode: bool,
    pub accent_color: Option<u32>,
}

impl UserSettings {
    pub fn defaults_for(user_id: Snowflake) -> Self {
        Self {
            user_id,
            default_delivery_mode: DeliveryMode::Standard,
            theme: Theme::Dark,
            locale: "en-US".to_owned(),
            status: PresenceStatus::Online,
            custom_status: None,
            developer_mode: false,
            accent_color: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn user(suspension: Option<Suspension>) -> User {
        User {
            id: Snowflake::new(1),
            username: "alice".into(),
            global_name: None,
            avatar: None,
            bot: false,
            system: false,
            email: Some("a@example.com".into()),
            verified: false,
            mfa_enabled: false,
            flags: 0,
            platform_admin: false,
            suspension,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn suspension_table() {
        let now = Utc::now();
        let cases = [
            ("never suspended", None, false),
            ("indefinite", Some(Suspension { reason: "spam".into(), until: None }), true),
            ("future", Some(Suspension { reason: "spam".into(), until: Some(now + Duration::hours(1)) }), true),
            ("expired", Some(Suspension { reason: "spam".into(), until: Some(now - Duration::hours(1)) }), false),
        ];
        for (name, suspension, expected) in cases {
            assert_eq!(user(suspension).is_suspended(now), expected, "{name}");
        }
    }

    #[test]
    fn display_name_prefers_global_name() {
        let mut u = user(None);
        assert_eq!(u.display_name(), "alice");
        u.global_name = Some("Alice".into());
        assert_eq!(u.display_name(), "Alice");
    }
}
