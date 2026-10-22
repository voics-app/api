//! Request bodies (P9). Unknown fields are ignored; `Option<Option<T>>`
//! distinguishes an absent field from an explicit `null`.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use crate::wire::{double_option, Id, Perm};

// --- auth ------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub totp: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize)]
pub struct MfaLoginRequest {
    pub ticket: String,
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct ForgotPasswordRequest {
    pub email: String,
}

#[derive(Debug, Deserialize)]
pub struct ResetPasswordRequest {
    pub token: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct VerifyEmailRequest {
    pub token: String,
}

/// Disable, delete and TOTP enrolment: the current password re-authenticates.
#[derive(Debug, Deserialize)]
pub struct PasswordConfirmationRequest {
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct VerifyTotpRequest {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct DisableTotpRequest {
    pub password: String,
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct BackupCodesVerificationRequest {
    pub password: String,
    #[serde(default)]
    pub regenerate: bool,
}

// --- devices ---------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateDeviceRequest {
    pub name: String,
    pub identity_key_b64: String,
}

#[derive(Debug, Deserialize)]
pub struct UploadKeyPackagesRequest {
    pub packages: Vec<String>,
}

// --- users -----------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub struct ModifyCurrentUserRequest {
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub global_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub avatar: Option<Option<String>>,
    /// Current password; required with `username`, `email` or `new_password`.
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub new_password: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CustomStatusRequest {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub emoji_name: Option<String>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct GuildFolderRequest {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub color: Option<u32>,
    pub guild_ids: Vec<Id>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ModifyUserSettingsRequest {
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub custom_status: Option<Option<CustomStatusRequest>>,
    #[serde(default)]
    pub default_delivery_mode: Option<String>,
    #[serde(default)]
    pub e2ee_calls_default: Option<bool>,
    #[serde(default)]
    pub developer_mode: Option<bool>,
    #[serde(default, deserialize_with = "double_option")]
    pub accent_color: Option<Option<u32>>,
    #[serde(default)]
    pub guild_folders: Option<Vec<GuildFolderRequest>>,
    #[serde(default)]
    pub guild_positions: Option<Vec<Id>>,
    #[serde(default)]
    pub message_display_compact: Option<bool>,
    #[serde(default)]
    pub preferences: Option<serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MuteConfigRequest {
    #[serde(default)]
    pub end_time: Option<DateTime<Utc>>,
    /// Seconds from now (900, 3600, 28800, 86400, ...) or -1 for until turned off.
    #[serde(default)]
    pub selected_time_window: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ChannelOverrideRequest {
    pub channel_id: Id,
    #[serde(default, deserialize_with = "double_option")]
    pub message_notifications: Option<Option<u8>>,
    #[serde(default)]
    pub muted: Option<bool>,
    #[serde(default, deserialize_with = "double_option")]
    pub mute_config: Option<Option<MuteConfigRequest>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ModifyGuildNotificationSettingsRequest {
    #[serde(default, deserialize_with = "double_option")]
    pub message_notifications: Option<Option<u8>>,
    #[serde(default)]
    pub suppress_everyone: Option<bool>,
    #[serde(default)]
    pub suppress_roles: Option<bool>,
    #[serde(default)]
    pub muted: Option<bool>,
    #[serde(default, deserialize_with = "double_option")]
    pub mute_config: Option<Option<MuteConfigRequest>>,
    #[serde(default)]
    pub hide_muted_channels: Option<bool>,
    #[serde(default)]
    pub channel_overrides: Option<Vec<ChannelOverrideRequest>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct PutNoteRequest {
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDmRequest {
    pub recipient_ids: Vec<Id>,
}

#[derive(Debug, Default, Deserialize)]
pub struct PutRelationshipRequest {
    #[serde(default, rename = "type")]
    pub kind: Option<u8>,
}

#[derive(Debug, Deserialize)]
pub struct SetDeliveryModeRequest {
    pub mode: Option<String>,
}

// --- guilds ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateGuildRequest {
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub voice_region: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ModifyGuildRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub icon: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub description: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub voice_region: Option<Option<String>>,
    #[serde(default)]
    pub verification_level: Option<u8>,
    #[serde(default)]
    pub default_message_notifications: Option<u8>,
    #[serde(default)]
    pub explicit_content_filter: Option<u8>,
    #[serde(default, deserialize_with = "double_option")]
    pub afk_channel_id: Option<Option<Id>>,
    #[serde(default)]
    pub afk_timeout: Option<u32>,
    #[serde(default, deserialize_with = "double_option")]
    pub system_channel_id: Option<Option<Id>>,
    #[serde(default)]
    pub owner_id: Option<Id>,
}

#[derive(Debug, Deserialize)]
pub struct OverwriteRequest {
    pub id: Id,
    #[serde(rename = "type")]
    pub kind: u8,
    #[serde(default)]
    pub allow: Option<Perm>,
    #[serde(default)]
    pub deny: Option<Perm>,
}

#[derive(Debug, Deserialize)]
pub struct CreateChannelRequest {
    pub name: String,
    #[serde(default, rename = "type")]
    pub kind: Option<u8>,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub bitrate: Option<u32>,
    #[serde(default)]
    pub user_limit: Option<u32>,
    #[serde(default)]
    pub rate_limit_per_user: Option<u32>,
    #[serde(default)]
    pub position: Option<i32>,
    #[serde(default)]
    pub permission_overwrites: Vec<OverwriteRequest>,
    #[serde(default)]
    pub parent_id: Option<Id>,
    #[serde(default)]
    pub nsfw: Option<bool>,
    #[serde(default)]
    pub rtc_region: Option<String>,
    #[serde(default)]
    pub delivery_mode: Option<String>,
    #[serde(default)]
    pub voice_region: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ModifyChannelRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub topic: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub icon: Option<Option<String>>,
    #[serde(default)]
    pub bitrate: Option<u32>,
    #[serde(default)]
    pub user_limit: Option<u32>,
    #[serde(default)]
    pub rate_limit_per_user: Option<u32>,
    #[serde(default)]
    pub position: Option<i32>,
    #[serde(default, deserialize_with = "double_option")]
    pub parent_id: Option<Option<Id>>,
    #[serde(default)]
    pub nsfw: Option<bool>,
    #[serde(default)]
    pub text_chat_enabled: Option<bool>,
    #[serde(default)]
    pub permission_overwrites: Option<Vec<OverwriteRequest>>,
    #[serde(default)]
    pub delivery_mode: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub voice_region: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub rtc_region: Option<Option<String>>,
    #[serde(default)]
    pub archived: Option<bool>,
    #[serde(default)]
    pub locked: Option<bool>,
    #[serde(default)]
    pub auto_archive_duration: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ChannelPositionRequest {
    pub id: Id,
    #[serde(default, deserialize_with = "double_option")]
    pub position: Option<Option<i32>>,
    #[serde(default, deserialize_with = "double_option")]
    pub parent_id: Option<Option<Id>>,
    #[serde(default, deserialize_with = "double_option")]
    pub lock_permissions: Option<Option<bool>>,
}

#[derive(Debug, Deserialize)]
pub struct EditOverwriteRequest {
    #[serde(default)]
    pub allow: Option<Perm>,
    #[serde(default)]
    pub deny: Option<Perm>,
    #[serde(rename = "type")]
    pub kind: u8,
}

#[derive(Debug, Default, Deserialize)]
pub struct CreateInviteRequest {
    #[serde(default)]
    pub max_age: Option<u32>,
    #[serde(default)]
    pub max_uses: Option<u32>,
    #[serde(default)]
    pub temporary: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateThreadRequest {
    pub name: String,
    #[serde(default, rename = "type")]
    pub kind: Option<u8>,
    #[serde(default)]
    pub auto_archive_duration: Option<u32>,
    #[serde(default)]
    pub invitable: Option<bool>,
    #[serde(default)]
    pub rate_limit_per_user: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RoleRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub permissions: Option<Perm>,
    #[serde(default)]
    pub color: Option<u32>,
    #[serde(default)]
    pub hoist: Option<bool>,
    #[serde(default)]
    pub mentionable: Option<bool>,
    #[serde(default, deserialize_with = "double_option")]
    pub unicode_emoji: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub struct RolePositionRequest {
    pub id: Id,
    pub position: i32,
}

#[derive(Debug, Default, Deserialize)]
pub struct ModifyMemberRequest {
    #[serde(default, deserialize_with = "double_option")]
    pub nick: Option<Option<String>>,
    #[serde(default)]
    pub roles: Option<Vec<Id>>,
    #[serde(default)]
    pub mute: Option<bool>,
    #[serde(default)]
    pub deaf: Option<bool>,
    #[serde(default, deserialize_with = "double_option")]
    pub communication_disabled_until: Option<Option<DateTime<Utc>>>,
    /// A voice channel to move the member to; `null` disconnects them from voice.
    #[serde(default, deserialize_with = "double_option")]
    pub channel_id: Option<Option<Id>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct CreateBanRequest {
    #[serde(default)]
    pub delete_message_seconds: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct CreateEmojiRequest {
    pub name: String,
    pub image: String,
    #[serde(default)]
    pub roles: Vec<Id>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ModifyEmojiRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub roles: Option<Vec<Id>>,
}

// --- applications ----------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateApplicationRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ModifyApplicationRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub icon: Option<Option<String>>,
    #[serde(default)]
    pub bot_public: Option<bool>,
    #[serde(default)]
    pub bot_require_code_grant: Option<bool>,
    #[serde(default)]
    pub flags: Option<u64>,
    #[serde(default, deserialize_with = "double_option")]
    pub interactions_endpoint_url: Option<Option<String>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ApplicationCommandRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<u8>,
    #[serde(default)]
    pub options: Option<Value>,
    #[serde(default, deserialize_with = "double_option")]
    pub default_member_permissions: Option<Option<Perm>>,
    #[serde(default)]
    pub dm_permission: Option<bool>,
    #[serde(default)]
    pub nsfw: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct InteractionResponseRequest {
    #[serde(rename = "type")]
    pub kind: u8,
    #[serde(default)]
    pub data: Option<Value>,
}

/// Discordy extension: how a user client invokes a command or component.
#[derive(Debug, Deserialize)]
pub struct CreateInteractionRequest {
    pub application_id: Id,
    #[serde(rename = "type")]
    pub kind: u8,
    pub channel_id: Id,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub message_id: Option<Id>,
}

#[derive(Debug, Deserialize)]
pub struct OAuth2AuthorizeRequest {
    pub client_id: Id,
    pub guild_id: Id,
    pub permissions: Perm,
    pub scope: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateWebhookRequest {
    pub name: String,
    #[serde(default)]
    pub avatar: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ModifyWebhookRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub avatar: Option<Option<String>>,
    #[serde(default)]
    pub channel_id: Option<Id>,
}
