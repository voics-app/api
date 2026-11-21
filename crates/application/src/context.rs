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

/// Default for `Settings::discovery_min_members`.
pub const DEFAULT_DISCOVERY_MIN_MEMBERS: u64 = 5;

/// Default for `Settings::discovery_min_guild_age`, in days.
pub const DEFAULT_DISCOVERY_MIN_AGE_DAYS: i64 = 7;

/// Default for `Settings::account_deletion_retention`, in days.
/// Product name used when `BRAND_NAME` is unset.
pub const DEFAULT_BRAND_NAME: &str = "Voics"; // brand-check: allow (the brand source)
pub const DEFAULT_ACCOUNT_DELETION_DAYS: i64 = 14;

#[derive(Debug, Clone)]
pub struct Settings {
    /// Both admission and the delivery worker are disabled until explicitly enabled.
    pub announcement_delivery_enabled: bool,
    pub access_token_ttl: Duration,
    pub refresh_token_ttl: Duration,
    /// Devices not seen for this long leave E2EE device listings and are pruned.
    pub device_max_idle: Duration,
    pub voice_regions: Vec<VoiceRegion>,
    pub default_voice_region: String,
    /// Public base URL of the api, used to build webhook URLs.
    pub public_api_base: String,
    /// Product name shown in emails and as the authenticator-app issuer (env `BRAND_NAME`).
    pub brand_name: String,
    /// Prefix of the links in account emails, e.g. `voics://`.
    pub app_link_base: String,
    /// How long a deleted account stays recoverable by logging in.
    pub account_deletion_retention: Duration,
    /// Members a guild needs before it can be listed in Discover.
    pub discovery_min_members: u64,
    /// How old a guild must be before it can be listed in Discover.
    pub discovery_min_guild_age: Duration,
    /// Most members a group DM can have, the creator included.
    pub group_dm_max_participants: usize,
}

impl Settings {
    pub fn discovery_rules(&self) -> domain::discovery::DiscoveryRules {
        domain::discovery::DiscoveryRules {
            min_members: self.discovery_min_members,
            min_age: self.discovery_min_guild_age,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            announcement_delivery_enabled: false,
            access_token_ttl: Duration::minutes(15),
            refresh_token_ttl: Duration::days(30),
            device_max_idle: Duration::days(DEFAULT_DEVICE_MAX_IDLE_DAYS),
            voice_regions: vec![VoiceRegion {
                id: "nl-home".into(),
                name: "Netherlands (Home)".into(),
            }],
            default_voice_region: "nl-home".into(),
            public_api_base: "http://localhost:8080".into(),
            brand_name: DEFAULT_BRAND_NAME.into(),
            app_link_base: "voics://".into(),
            account_deletion_retention: Duration::days(DEFAULT_ACCOUNT_DELETION_DAYS),
            discovery_min_members: DEFAULT_DISCOVERY_MIN_MEMBERS,
            discovery_min_guild_age: Duration::days(DEFAULT_DISCOVERY_MIN_AGE_DAYS),
            group_dm_max_participants: domain::channel::DEFAULT_MAX_GROUP_DM_PARTICIPANTS,
        }
    }
}

#[derive(Clone)]
pub struct Ports {
    pub connections: Option<Arc<dyn Connections>>,
    pub users: Arc<dyn UserRepository>,
    pub sessions: Arc<dyn SessionRepository>,
    pub account_tokens: Arc<dyn AccountTokenRepository>,
    pub devices: Arc<dyn DeviceRepository>,
    pub push_tokens: Arc<dyn PushTokenRepository>,
    pub relationships: Arc<dyn RelationshipRepository>,
    pub reports: Arc<dyn ReportRepository>,
    pub channels: Arc<dyn ChannelRepository>,
    pub thread_lifecycle: Arc<dyn ThreadLifecycleStore>,
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
    pub id_reservations: Arc<dyn IdReservations>,
    pub secrets: Arc<dyn SecretGenerator>,
    pub totp: Arc<dyn TotpVerifier>,
    pub passkeys: Arc<dyn PasskeyRepository>,
    pub webauthn: Option<Arc<dyn WebauthnVerifier>>,
    pub mailer: Arc<dyn Mailer>,
    /// Per-user and per-address throttles inside use cases (second-factor
    /// attempts, outbound mail); the HTTP layer keeps its own limiter.
    pub limiter: Arc<dyn RateLimiter>,
    pub presence: Arc<dyn PresenceDirectory>,
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
