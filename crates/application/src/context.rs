//! The dependencies use cases run against, wired at the composition root.

use std::sync::Arc;

use chrono::Duration;
use uuid::Uuid;

use domain::Snowflake;

use crate::error::{AppError, AppResult, Denial};
use crate::ports::*;

/// A voice region offered by `GET /voice/regions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceRegion {
    pub id: String,
    pub name: String,
}

/// Default for `Settings::device_max_idle`, in days.
pub const DEFAULT_DEVICE_MAX_IDLE_DAYS: i64 = 30;

/// Default for `Settings::account_deletion_retention`, in days.
pub const DEFAULT_ACCOUNT_DELETION_DAYS: i64 = 14;

#[derive(Debug, Clone)]
pub struct Settings {
    pub access_token_ttl: Duration,
    pub refresh_token_ttl: Duration,
    /// Devices not seen for this long leave E2EE device listings and are pruned.
    pub device_max_idle: Duration,
    pub voice_regions: Vec<VoiceRegion>,
    pub default_voice_region: String,
    /// Public base URL of the api, used to build webhook URLs.
    pub public_api_base: String,
    /// Prefix of the links in account emails, e.g. `discordy://`.
    pub app_link_base: String,
    /// How long a deleted account stays recoverable by logging in.
    pub account_deletion_retention: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            access_token_ttl: Duration::minutes(15),
            refresh_token_ttl: Duration::days(30),
            device_max_idle: Duration::days(DEFAULT_DEVICE_MAX_IDLE_DAYS),
            voice_regions: vec![VoiceRegion {
                id: "nl-home".into(),
                name: "Netherlands (Home)".into(),
            }],
            default_voice_region: "nl-home".into(),
            public_api_base: "http://localhost:8080".into(),
            app_link_base: "discordy://".into(),
            account_deletion_retention: Duration::days(DEFAULT_ACCOUNT_DELETION_DAYS),
        }
    }
}

#[derive(Clone)]
pub struct Ports {
    pub users: Arc<dyn UserRepository>,
    pub sessions: Arc<dyn SessionRepository>,
    pub account_tokens: Arc<dyn AccountTokenRepository>,
    pub devices: Arc<dyn DeviceRepository>,
    pub relationships: Arc<dyn RelationshipRepository>,
    pub channels: Arc<dyn ChannelRepository>,
    pub guilds: Arc<dyn GuildRepository>,
    pub invites: Arc<dyn InviteRepository>,
    pub emojis: Arc<dyn EmojiRepository>,
    pub audit_log: Arc<dyn AuditLogRepository>,
    pub applications: Arc<dyn ApplicationRepository>,
    pub interactions: Arc<dyn InteractionRepository>,
    pub webhooks: Arc<dyn WebhookRepository>,
    pub unit_of_work: Arc<dyn UnitOfWork>,
    pub events: Arc<dyn EventPublisher>,
    pub passwords: Arc<dyn PasswordHasher>,
    pub tokens: Arc<dyn TokenIssuer>,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdGenerator>,
    pub secrets: Arc<dyn SecretGenerator>,
    pub totp: Arc<dyn TotpVerifier>,
    pub mailer: Arc<dyn Mailer>,
    /// Per-user and per-address throttles inside use cases (second-factor
    /// attempts, outbound mail); the HTTP layer keeps its own limiter.
    pub limiter: Arc<dyn RateLimiter>,
    pub assets: Arc<dyn AssetStorage>,
    pub messages: Arc<dyn MessageService>,
    pub settings: Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorKind {
    User {
        session_id: Uuid,
        device_id: Option<Snowflake>,
    },
    Bot {
        application_id: Snowflake,
    },
}

/// The authenticated caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actor {
    pub user_id: Snowflake,
    pub kind: ActorKind,
}

impl Actor {
    pub fn is_bot(&self) -> bool {
        matches!(self.kind, ActorKind::Bot { .. })
    }

    pub fn session_id(&self) -> Option<Uuid> {
        match self.kind {
            ActorKind::User { session_id, .. } => Some(session_id),
            ActorKind::Bot { .. } => None,
        }
    }

    pub fn application_id(&self) -> Option<Snowflake> {
        match self.kind {
            ActorKind::Bot { application_id } => Some(application_id),
            ActorKind::User { .. } => None,
        }
    }

    /// Rejects bots for user-only endpoints (relationships, devices, ...).
    pub fn require_user(&self) -> AppResult<()> {
        if self.is_bot() {
            return Err(AppError::Forbidden(Denial::BotsNotAllowed));
        }
        Ok(())
    }
}
