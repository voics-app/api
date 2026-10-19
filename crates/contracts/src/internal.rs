//! `/internal/v1` payloads (P3, P4).

use serde::{Deserialize, Serialize};

use crate::objects::{ChannelDto, GuildFullDto, RelationshipDto, UserDto, UserSettingsDto};
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
