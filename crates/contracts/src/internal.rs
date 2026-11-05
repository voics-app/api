//! `/internal/v1` payloads (P3, P4).

use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use crate::objects::{
    ChannelDto, GuildFullDto, GuildNotificationSettingsDto, RelationshipDto, UserDto,
    UserSettingsDto,
};
use crate::wire::{Id, Perm};

#[derive(Debug, Clone, Deserialize)]
pub struct IntrospectRequest {
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IntrospectResponse {
    pub user_id: Id,
    pub kind: String,
    pub application_id: Option<Id>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UsersLookupRequest {
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserReadyDto {
    pub user: UserDto,
    pub guilds: Vec<GuildFullDto>,
    pub private_channels: Vec<ChannelDto>,
    pub relationships: Vec<RelationshipDto>,
    pub user_settings: UserSettingsDto,
    pub user_guild_settings: Vec<GuildNotificationSettingsDto>,
    /// The user's private notes keyed by the noted user's id.
    pub notes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelAccessDto {
    pub permissions: Perm,
    pub guild_id: Option<Id>,
    pub channel_type: u8,
    pub delivery_mode: String,
    pub recipient_ids: Vec<Id>,
}

/// `GET /internal/v1/channels/{id}/delivery`: the channel without a user.
#[derive(Debug, Clone, Serialize)]
pub struct ChannelRoutingDto {
    pub guild_id: Option<Id>,
    pub channel_type: u8,
    pub delivery_mode: String,
    pub recipient_ids: Vec<Id>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiAdminStatsDto {
    pub users_total: u64,
    pub guilds_total: u64,
    pub channels_total: u64,
    pub bots_total: u64,
    pub signups_24h: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminUserDto {
    pub id: Id,
    pub username: String,
    pub email: Option<String>,
    pub bot: bool,
    pub created_at: String,
    pub suspended_until: Option<String>,
    pub suspension_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminUserPageDto {
    pub users: Vec<AdminUserDto>,
    pub next_after: Option<Id>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminGuildDto {
    pub id: Id,
    pub name: String,
    pub owner_id: Id,
    pub member_count: u64,
    pub voice_region: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SuspendUserRequest {
    pub reason: String,
    #[serde(default)]
    pub until: Option<chrono::DateTime<chrono::Utc>>,
}

/// One push token of a target user.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PushTokenDto {
    pub platform: String,
    pub token: String,
    pub environment: String,
    pub previews: bool,
    pub only_mentions: bool,
}

/// A member the notifier may push to, with the facts its notification rules need.
#[derive(Debug, Clone, Serialize)]
pub struct PushTargetDto {
    pub user_id: Id,
    pub role_ids: Vec<Id>,
    /// The user holds a live gateway session.
    pub online: bool,
    /// Effective notification level for the channel: 0 all, 1 mentions, 2 nothing.
    pub level: u8,
    pub muted: bool,
    pub suppress_everyone: bool,
    pub suppress_roles: bool,
    pub tokens: Vec<PushTokenDto>,
}

/// `GET /internal/v1/channels/{id}/push-targets?author_id=`.
#[derive(Debug, Clone, Serialize)]
pub struct PushTargetsDto {
    pub guild_id: Option<Id>,
    pub guild_name: Option<String>,
    pub channel_name: Option<String>,
    pub delivery_mode: String,
    pub targets: Vec<PushTargetDto>,
}

/// `DELETE /internal/v1/push-tokens`: a token the provider rejected.
#[derive(Debug, Clone, Deserialize)]
pub struct ForgetPushTokenRequest {
    pub platform: String,
    pub token: String,
}
