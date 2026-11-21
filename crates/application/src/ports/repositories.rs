//! Read-side repository ports plus the few atomic operations that cannot be
//! expressed as a [`crate::write_set::WriteSet`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use domain::account::{AccountState, AccountToken, TokenPurpose};
use domain::application::{Application, ApplicationCommand, FollowedChannel, Interaction, Webhook};
use domain::audit::{AuditAction, AuditLogEntry};
use domain::bot_authorization::{BotAuthorization, BotInstallation, BotInstallationRevision};
use domain::channel::Channel;
use domain::delivery_mode::DeliveryMode;
use domain::device::Device;
use domain::discovery::GuildDiscovery;
use domain::emoji::Emoji;
use domain::guild::{Ban, Guild, Member, MemberProfile, Role};
use domain::invite::Invite;
use domain::mfa::BackupCodeRecord;
use domain::notification::GuildNotificationSettings;
use domain::push_token::{PushPlatform, PushToken, VoipToken};
use domain::relationship::Relationship;
use domain::report::{Report, ReportStatus};
use domain::session::ClientInfo;
use domain::template::GuildTemplate;
use domain::user::{Credentials, User, UserSettings};
use domain::Snowflake;

use crate::error::AppResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserStats {
    pub users_total: u64,
    pub bots_total: u64,
    pub signups_since: u64,
}

#[async_trait]
pub trait UserRepository: Send + Sync {
    /// Badge grants and visibility are independent of user flags.
    async fn badges(
        &self,
        _user_id: Snowflake,
        _include_hidden: bool,
    ) -> AppResult<Vec<(String, bool)>> {
        Err(crate::error::AppError::Unavailable(
            "Badge storage unavailable".into(),
        ))
    }
    /// Update an existing grant only; callers cannot grant themselves badges.
    async fn set_badge_visibility(
        &self,
        _user_id: Snowflake,
        _badge_id: &str,
        _visible: bool,
    ) -> AppResult<bool> {
        Err(crate::error::AppError::Unavailable(
            "Badge storage unavailable".into(),
        ))
    }
    async fn find(&self, id: Snowflake) -> AppResult<Option<User>>;
    async fn find_many(&self, ids: &[Snowflake]) -> AppResult<Vec<User>>;
    async fn find_by_email(&self, email: &str) -> AppResult<Option<User>>;
    async fn find_by_username(&self, username: &str) -> AppResult<Option<User>>;
    async fn credentials(&self, user_id: Snowflake) -> AppResult<Option<Credentials>>;
    /// Disabled / scheduled-for-deletion state; default when the user has none.
    async fn account_state(&self, user_id: Snowflake) -> AppResult<AccountState>;
    /// Every backup code of the user, oldest first, consumed ones included.
    async fn backup_codes(&self, user_id: Snowflake) -> AppResult<Vec<BackupCodeRecord>>;
    /// Marks one backup code consumed; `false` when it was already used.
    async fn consume_backup_code(&self, id: i64, now: DateTime<Utc>) -> AppResult<bool>;
    /// Users whose scheduled deletion is final at `now`, oldest first.
    async fn list_due_deletions(&self, now: DateTime<Utc>, limit: u32)
        -> AppResult<Vec<Snowflake>>;
    /// Stored settings, or defaults when none were saved yet.
    async fn settings(&self, user_id: Snowflake) -> AppResult<UserSettings>;
    async fn settings_many(&self, user_ids: &[Snowflake]) -> AppResult<Vec<UserSettings>>;
    /// Stored notification settings for one guild (`DIRECT_MESSAGES` for DMs), if any.
    async fn guild_notification_settings(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
    ) -> AppResult<Option<GuildNotificationSettings>>;
    /// Stored notification settings of several users for one guild (`DIRECT_MESSAGES` for DMs).
    async fn guild_notification_settings_many(
        &self,
        user_ids: &[Snowflake],
        guild_id: Snowflake,
    ) -> AppResult<Vec<GuildNotificationSettings>>;
    /// Every stored notification settings row of the user.
    async fn list_guild_notification_settings(
        &self,
        user_id: Snowflake,
    ) -> AppResult<Vec<GuildNotificationSettings>>;
    /// The private note `owner_id` wrote about `target_id`.
    async fn note(&self, owner_id: Snowflake, target_id: Snowflake) -> AppResult<Option<String>>;
    /// All notes of `owner_id` as `(target_id, note)`.
    async fn notes(&self, owner_id: Snowflake) -> AppResult<Vec<(Snowflake, String)>>;
    async fn search(
        &self,
        query: Option<&str>,
        limit: u32,
        after: Option<Snowflake>,
    ) -> AppResult<Vec<User>>;
    /// Users who share a DM, group DM or guild with `user_id` (may include `user_id`).
    async fn co_member_ids(&self, user_id: Snowflake) -> AppResult<Vec<Snowflake>>;
    async fn stats(&self, signups_since: DateTime<Utc>) -> AppResult<UserStats>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: Uuid,
    pub user_id: Snowflake,
    pub device_id: Option<Snowflake>,
    pub client: ClientInfo,
    pub created_at: DateTime<Utc>,
    /// Last login or token refresh.
    pub last_used_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// A live session with the name of the device it is bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveSession {
    pub session: Session,
    pub device_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RotateOutcome {
    Rotated(Session),
    /// The token was already rotated: the whole session family is revoked.
    ReuseDetected(Session),
    /// Unknown, expired or belonging to a revoked session.
    Invalid,
}

/// Only an identical request may recover the immediate, unused successor, and
/// only within this bounded interval. Repository implementations must not extend
/// the successor's expiry during recovery or store either plaintext credential.
pub struct RefreshReplay {
    pub request_hash: String,
    pub earliest_rotation: DateTime<Utc>,
}

#[async_trait]
/// Session revocation, including refresh-token reuse, atomically removes the session's
/// notification/VoIP tokens and durably queues `SESSION_REVOKE` for the gateway.
/// A failed commit must leave all three unchanged so a retry cannot miss a disconnect.
pub trait SessionRepository: Send + Sync {
    async fn create(
        &self,
        session: &Session,
        refresh_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> AppResult<()>;
    /// Atomically marks `old_hash` as rotated and stores `new_hash`; detects reuse.
    async fn rotate(
        &self,
        old_hash: &str,
        new_hash: &str,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
        replay: Option<&RefreshReplay>,
    ) -> AppResult<RotateOutcome>;
    async fn find(&self, id: Uuid) -> AppResult<Option<Session>>;
    async fn revoke(&self, id: Uuid, now: DateTime<Utc>) -> AppResult<()>;
    /// Revokes every live session of the user and returns their ids.
    async fn revoke_all_for_user(
        &self,
        user_id: Snowflake,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<Uuid>>;
    /// Revokes every live session of the user except `keep` and returns their ids.
    async fn revoke_others(
        &self,
        user_id: Snowflake,
        keep: Uuid,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<Uuid>>;
    /// The user's live sessions, most recently used first: not revoked, and
    /// with an unrotated refresh token that has not expired.
    async fn list_active(
        &self,
        user_id: Snowflake,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<ActiveSession>>;
    /// Binds a live session to its owner's device and moves any already registered
    /// notification token with it, atomically with respect to token writes/removal.
    async fn bind_device(&self, id: Uuid, device_id: Snowflake) -> AppResult<()>;
    /// Devices of `user_id` that a session bound to them still holds: not
    /// revoked, and with an unrotated refresh token that has not expired.
    async fn live_device_ids(
        &self,
        user_id: Snowflake,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<Snowflake>>;
}

/// Single-use tokens for email verification, password reset and login MFA tickets.
#[async_trait]
pub trait AccountTokenRepository: Send + Sync {
    async fn create(&self, token: &AccountToken) -> AppResult<()>;
    /// An unused, unexpired token, left in place.
    async fn find_active(
        &self,
        token_hash: &str,
        purpose: TokenPurpose,
        now: DateTime<Utc>,
    ) -> AppResult<Option<AccountToken>>;
    /// Atomically marks an unused, unexpired token used and returns it.
    async fn consume(
        &self,
        token_hash: &str,
        purpose: TokenPurpose,
        now: DateTime<Utc>,
    ) -> AppResult<Option<AccountToken>>;
    /// Voids the user's outstanding tokens of `purpose`.
    async fn revoke_for_user(
        &self,
        user_id: Snowflake,
        purpose: TokenPurpose,
        now: DateTime<Utc>,
    ) -> AppResult<()>;
}

#[async_trait]
pub trait DeviceRepository: Send + Sync {
    async fn find(&self, id: Snowflake) -> AppResult<Option<Device>>;
    async fn list_for_user(&self, user_id: Snowflake) -> AppResult<Vec<Device>>;
    /// Records that the device was seen at `now` (throttled by the adapter).
    async fn touch(&self, device_id: Snowflake, now: DateTime<Utc>) -> AppResult<()>;
    /// Devices last seen before `cutoff`, least recently seen first.
    async fn list_stale(&self, cutoff: DateTime<Utc>, limit: u32) -> AppResult<Vec<Device>>;
    async fn add_key_packages(&self, device_id: Snowflake, packages: &[Vec<u8>]) -> AppResult<()>;
    /// Removes and returns one package; concurrent claims never get the same one.
    async fn claim_key_package(&self, device_id: Snowflake) -> AppResult<Option<Vec<u8>>>;
}

#[async_trait]
pub trait RelationshipRepository: Send + Sync {
    async fn list(&self, user_id: Snowflake) -> AppResult<Vec<Relationship>>;
    async fn favourites(&self, user_id: Snowflake) -> AppResult<Vec<Snowflake>>;
    async fn set_favourite(
        &self,
        user_id: Snowflake,
        target_id: Snowflake,
        favourite: bool,
    ) -> AppResult<bool>;
    async fn create_friend_invite(
        &self,
        owner: Snowflake,
        hash: &str,
        expires_at: DateTime<Utc>,
    ) -> AppResult<()>;
    async fn friend_invite_owner(
        &self,
        hash: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<Snowflake>>;
    async fn revoke_friend_invites(&self, owner: Snowflake) -> AppResult<()>;
    async fn get(
        &self,
        user_id: Snowflake,
        target_id: Snowflake,
    ) -> AppResult<Option<Relationship>>;
}

#[async_trait]
pub trait ChannelRepository: Send + Sync {
    async fn find_forum_request(
        &self,
        guild_id: Snowflake,
        parent_id: Snowflake,
        owner_id: Snowflake,
        request_key: &str,
    ) -> AppResult<Option<Channel>> {
        Ok(self.list_for_guild(guild_id).await?.into_iter().find(|c| {
            c.parent_id == Some(parent_id)
                && c.owner_id == Some(owner_id)
                && c.forum_settings
                    .get("request_key")
                    .and_then(serde_json::Value::as_str)
                    == Some(request_key)
        }))
    }
    /// Marks a confirmed deleted opener as terminal only while this lease is current.
    async fn fail_forum_post(&self, _id: Snowflake, _token: &str) -> AppResult<bool> {
        Err(crate::error::AppError::Unavailable(
            "forum reservations unavailable".into(),
        ))
    }
    /// Claims or takes over an expired forum delivery/publication lease.
    async fn claim_forum_post(
        &self,
        _id: Snowflake,
        _phase: &str,
        _token: &str,
    ) -> AppResult<bool> {
        Err(crate::error::AppError::Unavailable(
            "forum reservations unavailable".into(),
        ))
    }

    async fn find(&self, id: Snowflake) -> AppResult<Option<Channel>>;
    async fn list_for_guild(&self, guild_id: Snowflake) -> AppResult<Vec<Channel>>;
    async fn list_private(&self, user_id: Snowflake) -> AppResult<Vec<Channel>>;
    async fn find_dm(&self, a: Snowflake, b: Snowflake) -> AppResult<Option<Channel>>;
    /// Per-chat overrides set by participants of `channel_id`.
    async fn delivery_overrides(
        &self,
        channel_id: Snowflake,
    ) -> AppResult<Vec<(Snowflake, DeliveryMode)>>;
    async fn count(&self) -> AppResult<u64>;
}

#[derive(Debug, Clone)]
pub struct ScheduledEventDelivery {
    pub id: uuid::Uuid,
    pub lease_id: uuid::Uuid,
    pub guild_id: Snowflake,
    pub event_id: Snowflake,
    pub user_id: Snowflake,
    pub session_id: uuid::Uuid,
    pub start: DateTime<Utc>,
    pub schedule_revision: i64,
}

#[async_trait]
pub trait GuildRepository: Send + Sync {
    async fn appeal_form(&self, guild_id: Snowflake) -> AppResult<Option<String>>;
    async fn set_appeal_form(&self, guild_id: Snowflake, question: &str) -> AppResult<()>;
    async fn submit_appeal(
        &self,
        id: Snowflake,
        guild_id: Snowflake,
        user_id: Snowflake,
        answer: &str,
        question: &str,
    ) -> AppResult<Option<domain::ban_appeal::BanAppeal>>;
    async fn appeal(
        &self,
        guild_id: Snowflake,
        id: Snowflake,
    ) -> AppResult<Option<domain::ban_appeal::BanAppeal>>;
    async fn appeals(&self, guild_id: Snowflake) -> AppResult<Vec<domain::ban_appeal::BanAppeal>>;
    async fn claim_scheduled_event_delivery(&self) -> AppResult<Option<ScheduledEventDelivery>>;
    async fn scheduled_event_delivery(
        &self,
        delivery_id: uuid::Uuid,
        lease_id: uuid::Uuid,
    ) -> AppResult<Option<ScheduledEventDelivery>>;
    async fn finish_scheduled_event_delivery(
        &self,
        delivery_id: uuid::Uuid,
        lease_id: uuid::Uuid,
        retry_after_secs: Option<u32>,
    ) -> AppResult<bool>;
    async fn scheduled_events(
        &self,
        guild_id: Snowflake,
    ) -> AppResult<Vec<domain::scheduled_event::ScheduledEvent>>;
    async fn scheduled_event(
        &self,
        guild_id: Snowflake,
        event_id: Snowflake,
    ) -> AppResult<Option<domain::scheduled_event::ScheduledEvent>>;
    async fn scheduled_event_interested(
        &self,
        event_id: Snowflake,
        user_id: Snowflake,
    ) -> AppResult<bool>;
    async fn onboarding(&self, guild_id: Snowflake) -> AppResult<(serde_json::Value, i64)>;
    async fn set_onboarding(
        &self,
        guild_id: Snowflake,
        value: &serde_json::Value,
    ) -> AppResult<i64>;
    async fn welcome_screen(&self, guild_id: Snowflake) -> AppResult<serde_json::Value>;
    async fn set_welcome_screen(
        &self,
        guild_id: Snowflake,
        value: &serde_json::Value,
    ) -> AppResult<()>;
    async fn screening(&self, guild_id: Snowflake) -> AppResult<ScreeningConfig>;
    async fn set_screening(
        &self,
        guild_id: Snowflake,
        enabled: bool,
        rules: &[String],
    ) -> AppResult<ScreeningConfig>;
    async fn find(&self, id: Snowflake) -> AppResult<Option<Guild>>;
    async fn widget_settings(&self, id: Snowflake) -> AppResult<Option<WidgetSettings>>;
    async fn public_widget(&self, id: Snowflake) -> AppResult<Option<PublicWidget>>;
    async fn membership_activity(
        &self,
        id: Snowflake,
        since: chrono::NaiveDate,
    ) -> AppResult<(DateTime<Utc>, Vec<MembershipActivity>)>;
    async fn voice_checkpoint(&self, checkpoint: VoiceCheckpoint) -> AppResult<()>;
    async fn update_widget_settings(
        &self,
        id: Snowflake,
        enabled: Option<bool>,
        channel: Option<Option<Snowflake>>,
    ) -> AppResult<WidgetSettings>;
    async fn list_for_user(&self, user_id: Snowflake) -> AppResult<Vec<Guild>>;
    async fn roles(&self, guild_id: Snowflake) -> AppResult<Vec<Role>>;
    async fn member(&self, guild_id: Snowflake, user_id: Snowflake) -> AppResult<Option<Member>>;
    async fn member_profiles(
        &self,
        guild_id: Snowflake,
        limit: u32,
        after: Option<Snowflake>,
    ) -> AppResult<Vec<MemberProfile>>;
    async fn members(&self, guild_id: Snowflake) -> AppResult<Vec<Member>>;
    async fn member_count(&self, guild_id: Snowflake) -> AppResult<u64>;
    /// Members a prune would remove: joined before `cutoff`, no login session used since
    /// `cutoff`, and no role besides `include_roles`. The owner and bots are never candidates.
    async fn prune_candidates(
        &self,
        guild_id: Snowflake,
        cutoff: DateTime<Utc>,
        include_roles: &[Snowflake],
    ) -> AppResult<Vec<Snowflake>>;
    /// Ids of up to `limit` members, lowest first.
    async fn member_ids(&self, guild_id: Snowflake, limit: u32) -> AppResult<Vec<Snowflake>>;
    async fn ban(&self, guild_id: Snowflake, user_id: Snowflake) -> AppResult<Option<Ban>>;
    async fn bans(&self, guild_id: Snowflake) -> AppResult<Vec<Ban>>;
    async fn search(&self, query: Option<&str>, limit: u32) -> AppResult<Vec<Guild>>;
    async fn count(&self) -> AppResult<u64>;
    /// The guild's discovery listing, if it opted in.
    async fn discovery(&self, guild_id: Snowflake) -> AppResult<Option<GuildDiscovery>>;
    /// Lists (`Some`, replacing any previous listing) or delists (`None`) the guild.
    async fn set_discovery(
        &self,
        guild_id: Snowflake,
        listing: Option<&GuildDiscovery>,
    ) -> AppResult<()>;
    async fn has_nsfw_channel(&self, guild_id: Snowflake) -> AppResult<bool>;
    /// Listed guilds that still meet the rules, largest first.
    async fn search_discoverable(&self, query: &DiscoveryQuery) -> AppResult<DiscoveryPage>;
    /// A shared (non built-in) template by code.
    async fn template(&self, code: &str) -> AppResult<Option<GuildTemplate>>;
    async fn templates_for_guild(&self, guild_id: Snowflake) -> AppResult<Vec<GuildTemplate>>;
    async fn insert_template(&self, template: &GuildTemplate) -> AppResult<()>;
    /// Replaces name, description, snapshot and `updated_at`.
    async fn update_template(&self, template: &GuildTemplate) -> AppResult<()>;
    async fn delete_template(&self, code: &str) -> AppResult<()>;
    async fn bump_template_usage(&self, code: &str) -> AppResult<()>;
}

#[derive(Debug, Clone, Copy)]
pub struct WidgetSettings {
    pub enabled: bool,
    pub channel_id: Option<Snowflake>,
}

#[derive(Debug, Clone)]
pub struct PublicWidget {
    pub id: Snowflake,
    pub name: String,
    pub icon: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MembershipActivity {
    pub day: chrono::NaiveDate,
    pub joins: i64,
    pub leaves: i64,
    pub voice_millis: i64,
}

#[derive(Debug, Clone)]
pub struct VoiceCheckpoint {
    pub guild_id: Snowflake,
    pub user_id: Snowflake,
    pub session_id: String,
    pub generation_id: Uuid,
    pub observed_at: DateTime<Utc>,
    pub connected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreeningConfig {
    pub enabled: bool,
    pub rules: Vec<String>,
    pub revision: i64,
}

/// Filters of `GET /discovery/guilds`; the rule thresholds are applied in SQL too,
/// so a guild that stopped qualifying disappears without being delisted.
#[derive(Debug, Clone)]
pub struct DiscoveryQuery {
    pub text: Option<String>,
    pub category: Option<String>,
    pub tag: Option<String>,
    pub min_members: u64,
    pub created_before: DateTime<Utc>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone)]
pub struct DiscoveryEntry {
    pub guild: Guild,
    pub discovery: GuildDiscovery,
    pub member_count: u64,
}

#[derive(Debug, Clone)]
pub struct DiscoveryPage {
    pub entries: Vec<DiscoveryEntry>,
    pub total: u64,
}

#[async_trait]
pub trait InviteRepository: Send + Sync {
    async fn find(&self, code: &str) -> AppResult<Option<Invite>>;
    async fn list_for_guild(&self, guild_id: Snowflake) -> AppResult<Vec<Invite>>;
    async fn vanity_for_guild(
        &self,
        guild_id: Snowflake,
    ) -> AppResult<Option<domain::vanity::VanityInvite>>;
    async fn vanity_by_code(&self, code: &str) -> AppResult<Option<domain::vanity::VanityInvite>>;
}

#[async_trait]
pub trait EmojiRepository: Send + Sync {
    async fn list(&self, guild_id: Snowflake) -> AppResult<Vec<Emoji>>;
    async fn find(&self, guild_id: Snowflake, emoji_id: Snowflake) -> AppResult<Option<Emoji>>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AuditLogQuery {
    pub user_id: Option<Snowflake>,
    /// Entries about this user, so the history of a banned or kicked member can be pulled up.
    pub target_id: Option<Snowflake>,
    pub action: Option<AuditAction>,
    pub before: Option<Snowflake>,
    pub limit: u32,
}

#[async_trait]
pub trait AuditLogRepository: Send + Sync {
    async fn prune_expired(&self, now: DateTime<Utc>) -> AppResult<u64>;
    async fn list(
        &self,
        guild_id: Snowflake,
        query: AuditLogQuery,
    ) -> AppResult<Vec<AuditLogEntry>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BotIdentity {
    pub user_id: Snowflake,
    pub application_id: Snowflake,
}

#[async_trait]
pub trait ApplicationRepository: Send + Sync {
    async fn find(&self, id: Snowflake) -> AppResult<Option<Application>>;
    async fn find_by_bot(&self, bot_user_id: Snowflake) -> AppResult<Option<Application>>;
    async fn list_for_owner(&self, owner_id: Snowflake) -> AppResult<Vec<Application>>;
    async fn bot_identity(&self, token_hash: &str) -> AppResult<Option<BotIdentity>>;
    async fn commands(
        &self,
        application_id: Snowflake,
        guild_id: Option<Snowflake>,
    ) -> AppResult<Vec<ApplicationCommand>>;
    async fn command(
        &self,
        application_id: Snowflake,
        command_id: Snowflake,
    ) -> AppResult<Option<ApplicationCommand>>;
    /// Applications whose bot is a member of `guild_id`.
    async fn installed_in_guild(&self, guild_id: Snowflake) -> AppResult<Vec<Application>>;
    async fn installation(
        &self,
        guild_id: Snowflake,
        application_id: Snowflake,
    ) -> AppResult<Option<BotInstallation>>;
    /// One consistent read of presence and last generation, retained after removal.
    async fn installation_revision(
        &self,
        guild_id: Snowflake,
        application_id: Snowflake,
    ) -> AppResult<BotInstallationRevision>;
    async fn authorizations(&self, user_id: Snowflake) -> AppResult<Vec<BotAuthorization>>;
    async fn authorization(
        &self,
        user_id: Snowflake,
        id: Snowflake,
    ) -> AppResult<Option<BotAuthorization>>;
}

#[async_trait]
pub trait InteractionRepository: Send + Sync {
    async fn find(&self, id: Snowflake) -> AppResult<Option<Interaction>>;
    async fn find_by_token(
        &self,
        application_id: Snowflake,
        token_hash: &str,
    ) -> AppResult<Option<Interaction>>;
}

#[async_trait]
pub trait WebhookRepository: Send + Sync {
    async fn find(&self, id: Snowflake) -> AppResult<Option<Webhook>>;
    async fn list_for_guild(&self, guild_id: Snowflake) -> AppResult<Vec<Webhook>>;
    async fn list_followed_for_guild(&self, guild_id: Snowflake)
        -> AppResult<Vec<FollowedChannel>>;
    async fn followers_of(&self, channel_id: Snowflake) -> AppResult<Vec<FollowedChannel>>;
}

/// Push tokens of the mobile app, one per login session.
#[async_trait]
pub trait PushTokenRepository: Send + Sync {
    /// Stores the session's token. It replaces the session's previous token and any other row
    /// holding the same provider token (the installation moved to this session), and the
    /// user's least recently refreshed tokens beyond the per-user limit are dropped.
    /// The write must revalidate/lock the live session and use its current device;
    /// a prior authorization or device snapshot may have raced logout/rebinding.
    async fn upsert(&self, token: &PushToken) -> AppResult<()>;
    /// Removes the session's token; `false` when it had none.
    async fn delete_for_session(&self, session_id: Uuid) -> AppResult<bool>;
    async fn delete_for_sessions(&self, session_ids: &[Uuid]) -> AppResult<()>;
    /// Removes a token the provider rejected; `false` when it was already gone.
    async fn delete_by_token(&self, platform: PushPlatform, token: &str) -> AppResult<bool>;
    /// Removes tokens not refreshed since `cutoff`; returns how many.
    async fn delete_stale(&self, cutoff: DateTime<Utc>) -> AppResult<u64>;
    /// The tokens of these users whose session is still live, ordered by user and session.
    async fn list_live_for_users(&self, user_ids: &[Snowflake]) -> AppResult<Vec<PushToken>>;
    /// The live tokens of the next `limit` users (ordered by id, strictly after `after`) who hold
    /// at least one, ordered by user and session. Pages the whole audience for a manual push.
    async fn list_live_page(
        &self,
        after: Option<Snowflake>,
        limit: u32,
    ) -> AppResult<Vec<PushToken>>;

    // PushKit (VoIP) tokens. They leave with the same events as the notification token of their
    // session: `delete_for_session`, `delete_for_sessions`, `delete_by_token` (APNs) and
    // `delete_stale` remove them too.

    /// Stores the session's VoIP token, replacing its previous one and any other row holding the
    /// same token. Revalidates/locks the live session in the write transaction.
    async fn upsert_voip(&self, token: &VoipToken) -> AppResult<()>;
    /// Removes only the session's VoIP token; `false` when it had none.
    async fn delete_voip_for_session(&self, session_id: Uuid) -> AppResult<bool>;
    /// The VoIP tokens of these users whose session is still live, ordered by user and session.
    async fn list_live_voip_for_users(&self, user_ids: &[Snowflake]) -> AppResult<Vec<VoipToken>>;
}

/// Reports of messages and users, read only by the admin portal.
#[async_trait]
pub trait ReportRepository: Send + Sync {
    /// Stores the report. `false` when the reporter already has an open report on the same
    /// target, which makes repeating a report a silent no-op.
    async fn insert(&self, report: &Report) -> AppResult<bool>;
    async fn find(&self, id: Snowflake) -> AppResult<Option<Report>>;
    /// Reports in `status`, newest first, strictly older than the `after` id when given.
    async fn list(
        &self,
        status: ReportStatus,
        limit: u32,
        after: Option<Snowflake>,
    ) -> AppResult<Vec<Report>>;
    /// Closes an open report; `false` when it is not open (already closed or unknown).
    async fn close(
        &self,
        id: Snowflake,
        status: ReportStatus,
        by: &str,
        note: Option<&str>,
        at: DateTime<Utc>,
    ) -> AppResult<bool>;
}
