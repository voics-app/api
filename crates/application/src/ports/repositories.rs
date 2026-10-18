//! Read-side repository ports plus the few atomic operations that cannot be
//! expressed as a [`crate::write_set::WriteSet`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use domain::application::{Application, ApplicationCommand, Interaction, Webhook};
use domain::audit::{AuditAction, AuditLogEntry};
use domain::channel::Channel;
use domain::delivery_mode::DeliveryMode;
use domain::device::Device;
use domain::emoji::Emoji;
use domain::guild::{Ban, Guild, Member, MemberProfile, Role};
use domain::invite::Invite;
use domain::relationship::Relationship;
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
    async fn find(&self, id: Snowflake) -> AppResult<Option<User>>;
    async fn find_many(&self, ids: &[Snowflake]) -> AppResult<Vec<User>>;
    async fn find_by_email(&self, email: &str) -> AppResult<Option<User>>;
    async fn find_by_username(&self, username: &str) -> AppResult<Option<User>>;
    async fn credentials(&self, user_id: Snowflake) -> AppResult<Option<Credentials>>;
    /// Stored settings, or defaults when none were saved yet.
    async fn settings(&self, user_id: Snowflake) -> AppResult<UserSettings>;
    async fn settings_many(&self, user_ids: &[Snowflake]) -> AppResult<Vec<UserSettings>>;
    async fn search(
        &self,
        query: Option<&str>,
        limit: u32,
        after: Option<Snowflake>,
    ) -> AppResult<Vec<User>>;
    async fn stats(&self, signups_since: DateTime<Utc>) -> AppResult<UserStats>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: Uuid,
    pub user_id: Snowflake,
    pub device_id: Option<Snowflake>,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RotateOutcome {
    Rotated(Session),
    /// The token was already rotated: the whole session family is revoked.
    ReuseDetected(Session),
    /// Unknown, expired or belonging to a revoked session.
    Invalid,
}

#[async_trait]
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
    ) -> AppResult<RotateOutcome>;
    async fn find(&self, id: Uuid) -> AppResult<Option<Session>>;
    async fn revoke(&self, id: Uuid, now: DateTime<Utc>) -> AppResult<()>;
    async fn revoke_all_for_user(&self, user_id: Snowflake, now: DateTime<Utc>) -> AppResult<()>;
    async fn bind_device(&self, id: Uuid, device_id: Snowflake) -> AppResult<()>;
}

#[async_trait]
pub trait DeviceRepository: Send + Sync {
    async fn create(&self, device: &Device) -> AppResult<()>;
    async fn find(&self, id: Snowflake) -> AppResult<Option<Device>>;
    async fn list_for_user(&self, user_id: Snowflake) -> AppResult<Vec<Device>>;
    async fn add_key_packages(&self, device_id: Snowflake, packages: &[Vec<u8>]) -> AppResult<()>;
    /// Removes and returns one package; concurrent claims never get the same one.
    async fn claim_key_package(&self, device_id: Snowflake) -> AppResult<Option<Vec<u8>>>;
}

#[async_trait]
pub trait RelationshipRepository: Send + Sync {
    async fn list(&self, user_id: Snowflake) -> AppResult<Vec<Relationship>>;
    async fn get(
        &self,
        user_id: Snowflake,
        target_id: Snowflake,
    ) -> AppResult<Option<Relationship>>;
}

#[async_trait]
pub trait ChannelRepository: Send + Sync {
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

#[async_trait]
pub trait GuildRepository: Send + Sync {
    async fn find(&self, id: Snowflake) -> AppResult<Option<Guild>>;
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
    async fn ban(&self, guild_id: Snowflake, user_id: Snowflake) -> AppResult<Option<Ban>>;
    async fn bans(&self, guild_id: Snowflake) -> AppResult<Vec<Ban>>;
    async fn search(&self, query: Option<&str>, limit: u32) -> AppResult<Vec<Guild>>;
    async fn count(&self) -> AppResult<u64>;
}

#[async_trait]
pub trait InviteRepository: Send + Sync {
    async fn find(&self, code: &str) -> AppResult<Option<Invite>>;
    async fn list_for_guild(&self, guild_id: Snowflake) -> AppResult<Vec<Invite>>;
}

#[async_trait]
pub trait EmojiRepository: Send + Sync {
    async fn list(&self, guild_id: Snowflake) -> AppResult<Vec<Emoji>>;
    async fn find(&self, guild_id: Snowflake, emoji_id: Snowflake) -> AppResult<Option<Emoji>>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AuditLogQuery {
    pub user_id: Option<Snowflake>,
    pub action: Option<AuditAction>,
    pub before: Option<Snowflake>,
    pub limit: u32,
}

#[async_trait]
pub trait AuditLogRepository: Send + Sync {
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
}
