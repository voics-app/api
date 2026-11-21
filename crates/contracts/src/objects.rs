//! Discord-shaped response objects (P9), built from domain entities.

use std::collections::BTreeMap;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::Serialize;
use serde_json::{json, Value};

use domain::application::{Application, ApplicationCommand, Interaction, Webhook};
use domain::audit::AuditLogEntry;
use domain::channel::{Channel, ThreadMetadata};
use domain::delivery_mode::DeliveryMode;
use domain::device::Device;
use domain::emoji::Emoji;
use domain::guild::{Ban, Guild, GuildSnapshot, Member, MemberProfile, Role};
use domain::invite::Invite;
use domain::notification::{ChannelOverride, GuildNotificationSettings, Mute, DIRECT_MESSAGES};
use domain::permissions::{PermissionOverwrite, Permissions};
use domain::relationship::Relationship;
use domain::sticker::Sticker;
use domain::user::{CustomStatus, User, UserSettings};
use domain::Snowflake;

use crate::wire::{timestamp, Id, Perm};

fn ids(values: &[Snowflake]) -> Vec<Id> {
    values.iter().copied().map(Id).collect()
}

fn base64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

// --- users -----------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct UserDto {
    pub id: Id,
    pub username: String,
    pub discriminator: &'static str,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
    pub banner: Option<String>,
    pub pronouns: Option<String>,
    pub theme_colors: Option<[u32; 2]>,
    pub accent_color: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    pub bot: bool,
    pub system: bool,
    pub flags: u64,
    pub public_flags: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mfa_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

impl UserDto {
    /// What other users may see.
    pub fn public(user: &User) -> Self {
        Self {
            id: Id(user.id),
            username: user.username.clone(),
            discriminator: "0",
            global_name: user.global_name.clone(),
            avatar: user.avatar.clone(),
            banner: user.banner.clone(),
            pronouns: user.pronouns.clone(),
            theme_colors: user.theme_colors,
            accent_color: user.accent_color,
            bio: user.bio.clone(),
            bot: user.bot,
            system: user.system,
            flags: user.flags,
            public_flags: user.flags,
            mfa_enabled: None,
            verified: None,
            email: None,
            locale: None,
        }
    }

    /// What the account owner sees (`/users/@me`, auth responses).
    pub fn private(user: &User, locale: &str) -> Self {
        Self {
            mfa_enabled: Some(user.mfa_enabled),
            verified: Some(user.verified),
            email: user.email.clone(),
            locale: Some(locale.to_owned()),
            ..Self::public(user)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CustomStatusDto {
    pub text: Option<String>,
    pub emoji_id: Option<Id>,
    pub emoji_name: Option<String>,
    pub expires_at: Option<String>,
}

impl From<&CustomStatus> for CustomStatusDto {
    fn from(status: &CustomStatus) -> Self {
        Self {
            text: status.text.clone(),
            emoji_id: None,
            emoji_name: status.emoji_name.clone(),
            expires_at: status.expires_at.map(timestamp),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GuildFolderDto {
    pub id: Option<i64>,
    pub name: Option<String>,
    pub color: Option<u32>,
    pub guild_ids: Vec<Id>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserSettingsDto {
    pub locale: String,
    pub theme: &'static str,
    pub status: &'static str,
    pub custom_status: Option<CustomStatusDto>,
    pub default_delivery_mode: &'static str,
    pub e2ee_calls_default: bool,
    pub developer_mode: bool,
    pub message_display_compact: bool,
    pub accent_color: Option<u32>,
    pub guild_folders: Vec<GuildFolderDto>,
    pub guild_positions: Vec<Id>,
    pub pinned_dm_channel_ids: Vec<Id>,
    pub pinned_channel_ids: BTreeMap<Id, Vec<Id>>,
    pub preferences: serde_json::Value,
}

impl From<&UserSettings> for UserSettingsDto {
    fn from(settings: &UserSettings) -> Self {
        Self {
            locale: settings.locale.clone(),
            theme: settings.theme.as_str(),
            status: settings.status.as_str(),
            custom_status: settings.custom_status.as_ref().map(CustomStatusDto::from),
            default_delivery_mode: settings.default_delivery_mode.as_str(),
            e2ee_calls_default: settings.e2ee_calls_default,
            developer_mode: settings.developer_mode,
            message_display_compact: settings.message_display_compact,
            accent_color: settings.accent_color,
            guild_folders: settings
                .guild_folders
                .iter()
                .map(|f| GuildFolderDto {
                    id: f.id,
                    name: f.name.clone(),
                    color: f.color,
                    guild_ids: f.guild_ids.iter().copied().map(Id::from).collect(),
                })
                .collect(),
            guild_positions: settings
                .guild_positions
                .iter()
                .copied()
                .map(Id::from)
                .collect(),
            pinned_dm_channel_ids: settings
                .pinned_dm_channel_ids
                .iter()
                .copied()
                .map(Id::from)
                .collect(),
            pinned_channel_ids: settings
                .pinned_channel_ids
                .iter()
                .map(|(guild, channels)| {
                    (
                        Id::from(*guild),
                        channels.iter().copied().map(Id::from).collect(),
                    )
                })
                .collect(),
            preferences: serde_json::Value::Object(settings.preferences.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MuteConfigDto {
    /// `null` means until turned off.
    pub end_time: Option<String>,
}

fn mute_config(mute: &Mute) -> Option<MuteConfigDto> {
    mute.muted.then(|| MuteConfigDto {
        end_time: mute.end_time.map(timestamp),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelOverrideDto {
    pub channel_id: Id,
    /// 0 all, 1 mentions, 2 nothing; `null` inherits the guild level.
    pub message_notifications: Option<u8>,
    pub muted: bool,
    pub mute_config: Option<MuteConfigDto>,
}

impl From<&ChannelOverride> for ChannelOverrideDto {
    fn from(o: &ChannelOverride) -> Self {
        Self {
            channel_id: Id(o.channel_id),
            message_notifications: o.level.map(|l| l.code()),
            muted: o.mute.muted,
            mute_config: mute_config(&o.mute),
        }
    }
}

/// `GET /users/@me/guilds/{id}/settings` and the `USER_GUILD_SETTINGS_UPDATE` payload.
#[derive(Debug, Clone, Serialize)]
pub struct GuildNotificationSettingsDto {
    /// `null` for the user's direct messages.
    pub guild_id: Option<Id>,
    /// 0 all, 1 mentions, 2 nothing; `null` uses the guild's default.
    pub message_notifications: Option<u8>,
    pub suppress_everyone: bool,
    pub suppress_roles: bool,
    pub muted: bool,
    pub mute_config: Option<MuteConfigDto>,
    pub hide_muted_channels: bool,
    pub channel_overrides: Vec<ChannelOverrideDto>,
}

impl From<&GuildNotificationSettings> for GuildNotificationSettingsDto {
    fn from(s: &GuildNotificationSettings) -> Self {
        Self {
            guild_id: (s.guild_id != DIRECT_MESSAGES).then_some(Id(s.guild_id)),
            message_notifications: s.level.map(|l| l.code()),
            suppress_everyone: s.suppress_everyone,
            suppress_roles: s.suppress_roles,
            muted: s.mute.muted,
            mute_config: mute_config(&s.mute),
            hide_muted_channels: s.hide_muted_channels,
            channel_overrides: s.channel_overrides.iter().map(Into::into).collect(),
        }
    }
}

/// `GET/PUT /users/@me/notes/{id}`.
#[derive(Debug, Clone, Serialize)]
pub struct NoteDto {
    pub user_id: Id,
    pub note_user_id: Id,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelationshipDto {
    pub id: Id,
    #[serde(rename = "type")]
    pub kind: u8,
    pub user: UserDto,
    pub since: String,
}

impl RelationshipDto {
    pub fn new(relationship: &Relationship, user: &User) -> Self {
        Self {
            id: Id(relationship.target_id),
            kind: relationship.kind.code(),
            user: UserDto::public(user),
            since: timestamp(relationship.since),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MutualGuildDto {
    pub id: Id,
    pub nick: Option<String>,
}

/// `GET /users/{id}/profile`.
#[derive(Debug, Clone, Serialize)]
pub struct UserProfileDto {
    pub user: UserDto,
    pub mutual_guilds: Vec<MutualGuildDto>,
    pub mutual_friends: Vec<UserDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceDto {
    pub device_id: Id,
    pub identity_key_b64: String,
}

impl From<&Device> for DeviceDto {
    fn from(device: &Device) -> Self {
        Self {
            device_id: Id(device.id),
            identity_key_b64: base64(&device.identity_key),
        }
    }
}

/// A login session of the current user (`GET /users/@me/sessions`).
#[derive(Debug, Clone, Serialize)]
pub struct AuthSessionDto {
    pub id: String,
    pub client: String,
    pub os: String,
    pub device_name: Option<String>,
    pub location: Option<String>,
    pub created_at: String,
    pub last_used_at: String,
    pub current: bool,
}

// --- channels --------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct PermissionOverwriteDto {
    pub id: Id,
    #[serde(rename = "type")]
    pub kind: u8,
    pub allow: Perm,
    pub deny: Perm,
}

impl From<&PermissionOverwrite> for PermissionOverwriteDto {
    fn from(overwrite: &PermissionOverwrite) -> Self {
        Self {
            id: Id(overwrite.id),
            kind: overwrite.kind.code(),
            allow: Perm(overwrite.allow),
            deny: Perm(overwrite.deny),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadMetadataDto {
    pub archived: bool,
    pub auto_archive_duration: u32,
    pub archive_timestamp: String,
    pub locked: bool,
    pub invitable: bool,
    pub create_timestamp: String,
}

impl From<&ThreadMetadata> for ThreadMetadataDto {
    fn from(meta: &ThreadMetadata) -> Self {
        Self {
            archived: meta.archived,
            auto_archive_duration: meta.auto_archive_duration,
            archive_timestamp: timestamp(meta.archive_timestamp),
            locked: meta.locked,
            invitable: meta.invitable,
            create_timestamp: timestamp(meta.create_timestamp),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelDto {
    pub id: Id,
    #[serde(rename = "type")]
    pub kind: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<Id>,
    pub position: i32,
    pub permission_overwrites: Vec<PermissionOverwriteDto>,
    pub name: Option<String>,
    pub topic: Option<String>,
    pub nsfw: bool,
    pub text_chat_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_limit: Option<u32>,
    pub rate_limit_per_user: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipients: Option<Vec<UserDto>>,
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<Id>,
    pub parent_id: Option<Id>,
    pub rtc_region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_metadata: Option<ThreadMetadataDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_thread_member: Option<bool>,
    pub delivery_mode: &'static str,
    pub voice_region: Option<String>,
}

impl ChannelDto {
    fn build(
        channel: &Channel,
        delivery_mode: DeliveryMode,
        recipients: Option<Vec<UserDto>>,
    ) -> Self {
        Self {
            id: Id(channel.id),
            kind: channel.kind.code(),
            guild_id: channel.guild_id.map(Id),
            position: channel.position,
            permission_overwrites: channel
                .overwrites
                .iter()
                .map(PermissionOverwriteDto::from)
                .collect(),
            name: channel.name.clone(),
            topic: channel.topic.clone(),
            nsfw: channel.nsfw,
            text_chat_enabled: channel.text_chat_enabled,
            bitrate: channel.bitrate,
            user_limit: channel.user_limit,
            rate_limit_per_user: channel.rate_limit_per_user,
            recipients,
            icon: channel.icon.clone(),
            owner_id: channel.owner_id.map(Id),
            parent_id: channel.parent_id.map(Id),
            rtc_region: channel.voice_region.clone(),
            thread_metadata: channel
                .thread_metadata
                .as_ref()
                .map(ThreadMetadataDto::from),
            member_count: channel
                .kind
                .is_thread()
                .then_some(channel.thread_member_ids.len()),
            is_thread_member: None,
            delivery_mode: delivery_mode.as_str(),
            voice_region: channel.voice_region.clone(),
        }
    }

    pub fn guild(channel: &Channel) -> Self {
        Self::build(channel, channel.delivery_mode, None)
    }

    pub fn guild_for_viewer(channel: &Channel, viewer: Snowflake) -> Self {
        let mut dto = Self::guild(channel);
        dto.is_thread_member = channel
            .kind
            .is_thread()
            .then(|| channel.thread_member_ids.contains(&viewer));
        dto
    }

    /// A DM or group DM as seen by `viewer`, who is left out of `recipients`.
    pub fn private(
        channel: &Channel,
        recipients: &[User],
        delivery_mode: DeliveryMode,
        viewer: Option<Snowflake>,
    ) -> Self {
        let others = recipients
            .iter()
            .filter(|u| Some(u.id) != viewer)
            .map(UserDto::public)
            .collect();
        Self::build(channel, delivery_mode, Some(others))
    }
}

// --- guilds ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct RoleDto {
    pub id: Id,
    pub name: String,
    pub color: u32,
    pub hoist: bool,
    pub icon: Option<String>,
    pub unicode_emoji: Option<String>,
    pub position: i32,
    pub permissions: Perm,
    pub managed: bool,
    pub mentionable: bool,
    pub flags: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Value>,
}

impl From<&Role> for RoleDto {
    fn from(role: &Role) -> Self {
        Self {
            id: Id(role.id),
            name: role.name.clone(),
            color: role.color,
            hoist: role.hoist,
            icon: role.icon.clone(),
            unicode_emoji: role.unicode_emoji.clone(),
            position: role.position,
            permissions: Perm(role.permissions),
            managed: role.managed,
            mentionable: role.mentionable,
            flags: 0,
            tags: role
                .bot_id
                .map(|bot_id| json!({ "bot_id": bot_id.to_string() })),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StickerDto {
    pub id: Id,
    pub guild_id: Id,
    pub name: String,
    pub tags: String,
    pub format_type: i16,
    pub available: bool,
}

impl From<&Sticker> for StickerDto {
    fn from(sticker: &Sticker) -> Self {
        Self {
            id: Id(sticker.id),
            guild_id: Id(sticker.guild_id),
            name: sticker.name.clone(),
            tags: sticker.tags.clone(),
            format_type: sticker.format_type,
            available: sticker.available,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EmojiDto {
    pub id: Id,
    pub name: String,
    pub roles: Vec<Id>,
    pub require_colons: bool,
    pub managed: bool,
    pub animated: bool,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<UserDto>,
}

impl From<&Emoji> for EmojiDto {
    fn from(emoji: &Emoji) -> Self {
        Self {
            id: Id(emoji.id),
            name: emoji.name.clone(),
            roles: ids(&emoji.role_ids),
            require_colons: emoji.require_colons,
            managed: emoji.managed,
            animated: emoji.animated,
            available: emoji.available,
            user: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GuildDto {
    pub id: Id,
    pub name: String,
    pub icon: Option<String>,
    pub splash: Option<String>,
    pub discovery_splash: Option<String>,
    pub owner_id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Perm>,
    pub afk_channel_id: Option<Id>,
    pub afk_timeout: u32,
    pub verification_level: u8,
    pub default_message_notifications: u8,
    pub explicit_content_filter: u8,
    pub roles: Vec<RoleDto>,
    pub emojis: Vec<EmojiDto>,
    pub features: Vec<String>,
    pub rules_channel_id: Option<Id>,
    pub public_updates_channel_id: Option<Id>,
    pub mfa_level: u8,
    pub system_channel_id: Option<Id>,
    pub system_channel_flags: u32,
    pub description: Option<String>,
    pub banner: Option<String>,
    pub preferred_locale: &'static str,
    pub nsfw_level: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approximate_member_count: Option<u64>,
    pub voice_region: String,
    pub prune_admin_only: bool,
    pub audit_log_enabled: bool,
    pub audit_log_retention_days: u16,
}

impl GuildDto {
    pub fn new(guild: &Guild, roles: &[Role], emojis: &[Emoji], default_region: &str) -> Self {
        Self {
            id: Id(guild.id),
            name: guild.name.clone(),
            icon: guild.icon.clone(),
            splash: guild.splash.clone(),
            discovery_splash: None,
            owner_id: Id(guild.owner_id),
            owner: None,
            permissions: None,
            afk_channel_id: guild.afk_channel_id.map(Id),
            afk_timeout: guild.afk_timeout,
            verification_level: guild.verification_level,
            default_message_notifications: guild.default_message_notifications,
            explicit_content_filter: guild.explicit_content_filter,
            roles: roles.iter().map(RoleDto::from).collect(),
            emojis: emojis.iter().map(EmojiDto::from).collect(),
            features: guild
                .community
                .map(|_| vec!["COMMUNITY".into()])
                .unwrap_or_default(),
            rules_channel_id: guild.community.map(|c| Id(c.rules_channel_id)),
            public_updates_channel_id: guild.community.map(|c| Id(c.public_updates_channel_id)),
            mfa_level: 0,
            system_channel_id: guild.system_channel_id.map(Id),
            system_channel_flags: guild.system_channel_flags,
            description: guild.description.clone(),
            banner: guild.banner.clone(),
            preferred_locale: "en-US",
            nsfw_level: 0,
            approximate_member_count: None,
            voice_region: guild
                .voice_region
                .clone()
                .unwrap_or_else(|| default_region.to_owned()),
            prune_admin_only: guild.prune_admin_only,
            audit_log_enabled: guild.audit_policy.enabled,
            audit_log_retention_days: guild.audit_policy.retention_days,
        }
    }

    /// A guild without roles and emojis, as listed by `GET /users/@me/guilds`.
    pub fn partial(guild: &Guild, permissions: Permissions, default_region: &str) -> Self {
        Self {
            owner: None,
            permissions: Some(Perm(permissions)),
            ..Self::new(guild, &[], &[], default_region)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MemberDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<UserDto>,
    pub nick: Option<String>,
    pub avatar: Option<String>,
    pub banner: Option<String>,
    pub bio: Option<String>,
    pub roles: Vec<Id>,
    pub joined_at: String,
    pub deaf: bool,
    pub mute: bool,
    pub flags: u32,
    pub pending: bool,
    pub communication_disabled_until: Option<String>,
}

impl MemberDto {
    pub fn new(member: &Member, user: Option<&User>) -> Self {
        Self {
            user: user.map(UserDto::public),
            nick: member.nick.clone(),
            avatar: member.avatar.clone(),
            banner: member.banner.clone(),
            bio: member.bio.clone(),
            roles: ids(&member.role_ids),
            joined_at: timestamp(member.joined_at),
            deaf: member.deaf,
            mute: member.mute,
            flags: 0,
            pending: member.pending,
            communication_disabled_until: member.communication_disabled_until.map(timestamp),
        }
    }
}

impl From<&MemberProfile> for MemberDto {
    fn from(profile: &MemberProfile) -> Self {
        Self::new(&profile.member, Some(&profile.user))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GuildFullDto {
    #[serde(flatten)]
    pub guild: GuildDto,
    pub large: bool,
    pub member_count: u64,
    pub members: Vec<MemberDto>,
    pub channels: Vec<ChannelDto>,
    pub threads: Vec<ChannelDto>,
    pub voice_states: Vec<Value>,
}

impl GuildFullDto {
    pub fn new(snapshot: &GuildSnapshot, default_region: &str) -> Self {
        let (threads, channels): (Vec<&Channel>, Vec<&Channel>) =
            snapshot.channels.iter().partition(|c| c.kind.is_thread());
        Self {
            guild: GuildDto::new(
                &snapshot.guild,
                &snapshot.roles,
                &snapshot.emojis,
                default_region,
            ),
            large: snapshot.member_count > 250,
            member_count: snapshot.member_count,
            members: snapshot.members.iter().map(MemberDto::from).collect(),
            channels: channels.into_iter().map(ChannelDto::guild).collect(),
            threads: threads.into_iter().map(ChannelDto::guild).collect(),
            voice_states: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BanDto {
    pub reason: Option<String>,
    pub user: UserDto,
}

impl From<&Ban> for BanDto {
    fn from(ban: &Ban) -> Self {
        Self {
            reason: ban.reason.clone(),
            user: UserDto::public(&ban.user),
        }
    }
}

/// Result of both prune endpoints: members to be removed (GET) or removed (POST).
#[derive(Debug, Clone, Serialize)]
pub struct PruneDto {
    pub pruned: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PartialChannelDto {
    pub id: Id,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct InviteGuildDto {
    pub id: Id,
    pub name: String,
    pub icon: Option<String>,
    pub description: Option<String>,
    pub verification_level: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct InviteDto {
    #[serde(rename = "type")]
    pub kind: u8,
    pub code: String,
    pub guild: InviteGuildDto,
    pub channel: PartialChannelDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inviter: Option<UserDto>,
    pub expires_at: Option<String>,
    pub uses: u32,
    pub max_uses: u32,
    pub max_age: u32,
    pub temporary: bool,
    pub created_at: String,
    pub approximate_member_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approximate_presence_count: Option<u64>,
}

impl InviteDto {
    pub fn new(
        invite: &Invite,
        guild: &Guild,
        channel: &Channel,
        inviter: Option<&User>,
        member_count: u64,
        presence_count: Option<u64>,
    ) -> Self {
        Self {
            kind: 0,
            code: invite.code.clone(),
            guild: InviteGuildDto {
                id: Id(guild.id),
                name: guild.name.clone(),
                icon: guild.icon.clone(),
                description: guild.description.clone(),
                verification_level: guild.verification_level,
            },
            channel: PartialChannelDto {
                id: Id(channel.id),
                name: channel.name.clone(),
                kind: channel.kind.code(),
            },
            inviter: inviter.map(UserDto::public),
            expires_at: invite.expires_at().map(timestamp),
            uses: invite.uses,
            max_uses: invite.max_uses,
            max_age: invite.max_age,
            temporary: invite.temporary,
            created_at: timestamp(invite.created_at),
            approximate_member_count: member_count,
            approximate_presence_count: presence_count,
        }
    }
}

// --- audit log -------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct AuditLogEntryDto {
    pub id: Id,
    pub target_id: Option<String>,
    pub changes: Vec<Value>,
    pub user_id: Option<Id>,
    pub action_type: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl From<&AuditLogEntry> for AuditLogEntryDto {
    fn from(entry: &AuditLogEntry) -> Self {
        let changes = entry
            .changes
            .iter()
            .map(|change| {
                let mut object = serde_json::Map::new();
                object.insert("key".into(), Value::String(change.key.clone()));
                if let Some(old) = &change.old_value {
                    object.insert("old_value".into(), old.clone());
                }
                if let Some(new) = &change.new_value {
                    object.insert("new_value".into(), new.clone());
                }
                Value::Object(object)
            })
            .collect();
        Self {
            id: Id(entry.id),
            target_id: entry.target_id.map(|id| id.to_string()),
            changes,
            user_id: entry.user_id.map(Id),
            action_type: entry.action.code(),
            reason: entry.reason.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditLogDto {
    pub application_commands: Vec<Value>,
    pub audit_log_entries: Vec<AuditLogEntryDto>,
    pub threads: Vec<ChannelDto>,
    pub users: Vec<UserDto>,
    pub webhooks: Vec<Value>,
}

impl AuditLogDto {
    pub fn new(entries: &[AuditLogEntry], users: &[User]) -> Self {
        Self {
            application_commands: Vec::new(),
            audit_log_entries: entries.iter().map(AuditLogEntryDto::from).collect(),
            threads: Vec::new(),
            users: users.iter().map(UserDto::public).collect(),
            webhooks: Vec::new(),
        }
    }
}

// --- applications ----------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ApplicationDto {
    pub id: Id,
    pub name: String,
    pub icon: Option<String>,
    pub description: String,
    pub bot_public: bool,
    pub bot_require_code_grant: bool,
    pub bot_id: Id,
    pub bot: UserDto,
    pub owner: UserDto,
    pub verify_key: String,
    pub flags: u64,
    pub interactions_endpoint_url: Option<String>,
}

impl ApplicationDto {
    pub fn new(application: &Application, bot: &User, owner: &User) -> Self {
        Self {
            id: Id(application.id),
            name: application.name.clone(),
            icon: application.icon.clone(),
            description: application.description.clone(),
            bot_public: application.bot_public,
            bot_require_code_grant: application.bot_require_code_grant,
            bot_id: Id(application.bot_user_id),
            bot: UserDto::public(bot),
            owner: UserDto::public(owner),
            verify_key: application.verify_key.clone(),
            flags: application.flags,
            interactions_endpoint_url: application.interactions_endpoint_url.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplicationIndexDto {
    pub id: Id,
    pub name: String,
    pub icon: Option<String>,
    pub bot_id: Id,
}

impl From<&Application> for ApplicationIndexDto {
    fn from(application: &Application) -> Self {
        Self {
            id: Id(application.id),
            name: application.name.clone(),
            icon: application.icon.clone(),
            bot_id: Id(application.bot_user_id),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandIndexDto {
    pub applications: Vec<ApplicationIndexDto>,
    pub application_commands: Vec<ApplicationCommandDto>,
}

impl CommandIndexDto {
    pub fn new(applications: &[Application], commands: &[ApplicationCommand]) -> Self {
        Self {
            applications: applications.iter().map(ApplicationIndexDto::from).collect(),
            application_commands: commands.iter().map(ApplicationCommandDto::from).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplicationCommandDto {
    pub id: Id,
    #[serde(rename = "type")]
    pub kind: u8,
    pub application_id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<Id>,
    pub name: String,
    pub description: String,
    pub options: Value,
    pub default_member_permissions: Option<Perm>,
    pub dm_permission: bool,
    pub nsfw: bool,
    pub version: Id,
}

impl From<&ApplicationCommand> for ApplicationCommandDto {
    fn from(command: &ApplicationCommand) -> Self {
        Self {
            id: Id(command.id),
            kind: command.kind.code(),
            application_id: Id(command.application_id),
            guild_id: command.guild_id.map(Id),
            name: command.name.clone(),
            description: command.description.clone(),
            options: command.options.clone(),
            default_member_permissions: command.default_member_permissions.map(Perm),
            dm_permission: command.dm_permission,
            nsfw: command.nsfw,
            version: Id(command.version),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WebhookDto {
    pub id: Id,
    #[serde(rename = "type")]
    pub kind: u8,
    pub guild_id: Option<Id>,
    pub channel_id: Option<Id>,
    pub name: Option<String>,
    pub avatar: Option<String>,
    pub application_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl WebhookDto {
    /// `secret` is the raw token and the API base URL; known only at creation.
    pub fn new(webhook: &Webhook, secret: Option<(&str, &str)>) -> Self {
        Self {
            id: Id(webhook.id),
            kind: webhook.kind.code(),
            guild_id: Some(Id(webhook.guild_id)),
            channel_id: Some(Id(webhook.channel_id)),
            name: Some(webhook.name.clone()),
            avatar: webhook.avatar.clone(),
            application_id: webhook.application_id.map(Id),
            token: secret.map(|(_, token)| token.to_owned()),
            url: secret
                .map(|(base, token)| format!("{base}/api/v1/webhooks/{}/{token}", webhook.id)),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct VoiceRegionDto {
    pub id: String,
    pub name: String,
    pub optimal: bool,
}

/// INTERACTION_CREATE `data` (the gateway DISPATCH payload).
pub fn interaction_event(
    interaction: &Interaction,
    token: &str,
    app_permissions: Permissions,
    user: &User,
    member: Option<&MemberProfile>,
) -> Value {
    let mut object = json!({
        "id": interaction.id.to_string(),
        "application_id": interaction.application_id.to_string(),
        "type": interaction.kind.code(),
        "data": interaction.data,
        "channel_id": interaction.channel_id.to_string(),
        "token": token,
        "version": 1,
        "app_permissions": app_permissions.bits().to_string(),
        "locale": "en-US",
    });
    let fields = object.as_object_mut().expect("json! object");
    match (interaction.guild_id, member) {
        (Some(guild_id), Some(profile)) => {
            fields.insert("guild_id".into(), json!(guild_id.to_string()));
            fields.insert("guild".into(), json!({ "id": guild_id.to_string() }));
            fields.insert("member".into(), json!(MemberDto::from(profile)));
            fields.insert("context".into(), json!(0));
        }
        _ => {
            fields.insert("user".into(), json!(UserDto::public(user)));
            fields.insert("context".into(), json!(1));
        }
    }
    if let Some(message_id) = interaction.source_message_id {
        fields.insert("message".into(), json!({ "id": message_id.to_string() }));
    }
    object
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use domain::channel::ChannelType;

    fn user() -> User {
        User {
            id: Snowflake::new(10),
            username: "alice".into(),
            global_name: None,
            avatar: None,
            bio: None,
            banner: None,
            pronouns: None,
            theme_colors: None,
            accent_color: None,
            bot: false,
            system: false,
            email: Some("alice@example.com".into()),
            verified: true,
            mfa_enabled: false,
            flags: 0,
            platform_admin: false,
            suspension: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn public_users_hide_private_fields() {
        let public = serde_json::to_value(UserDto::public(&user())).unwrap();
        assert_eq!(public["id"], "10");
        assert_eq!(public["discriminator"], "0");
        assert!(public.get("email").is_none());
        let private = serde_json::to_value(UserDto::private(&user(), "nl")).unwrap();
        assert_eq!(private["email"], "alice@example.com");
        assert_eq!(private["locale"], "nl");
    }

    #[test]
    fn base64_matches_the_standard_alphabet() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn private_channels_leave_the_viewer_out_of_recipients() {
        let mut other = user();
        other.id = Snowflake::new(11);
        let mut channel = Channel::new(Snowflake::new(1), ChannelType::Dm, None, Utc::now());
        channel.recipient_ids = vec![Snowflake::new(10), Snowflake::new(11)];
        let dto = ChannelDto::private(
            &channel,
            &[user(), other],
            DeliveryMode::Ephemeral,
            Some(Snowflake::new(10)),
        );
        let json = serde_json::to_value(dto).unwrap();
        assert_eq!(json["type"], 1);
        assert_eq!(json["delivery_mode"], "ephemeral");
        assert_eq!(json["recipients"].as_array().unwrap().len(), 1);
        assert_eq!(json["recipients"][0]["id"], "11");
        assert!(json.get("guild_id").is_none());
    }

    #[test]
    fn webhook_secret_appears_only_when_provided() {
        let webhook = Webhook {
            id: Snowflake::new(5),
            kind: domain::application::WebhookKind::Incoming,
            guild_id: Snowflake::new(1),
            channel_id: Snowflake::new(2),
            creator_id: None,
            name: "ci".into(),
            avatar: None,
            application_id: None,
            token_hash: "hash".into(),
        };
        let hidden = serde_json::to_value(WebhookDto::new(&webhook, None)).unwrap();
        assert!(hidden.get("token").is_none());
        let shown =
            serde_json::to_value(WebhookDto::new(&webhook, Some(("https://api.test", "tok"))))
                .unwrap();
        assert_eq!(shown["url"], "https://api.test/api/v1/webhooks/5/tok");
    }
}

// --- discovery -------------------------------------------------------------

/// `GET /guilds/{id}/discovery` and `PUT /guilds/{id}/discovery`.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoverySettingsDto {
    pub enabled: bool,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub listed_at: Option<String>,
    pub member_count: u64,
    /// Codes of unmet listing rules; empty when the guild may be listed.
    pub blockers: Vec<String>,
}

impl DiscoverySettingsDto {
    pub fn new(
        listing: Option<&domain::discovery::GuildDiscovery>,
        blockers: &[domain::discovery::Ineligibility],
        member_count: u64,
    ) -> Self {
        Self {
            enabled: listing.is_some(),
            category: listing.map(|l| l.category.clone()),
            tags: listing.map(|l| l.tags.clone()).unwrap_or_default(),
            listed_at: listing.map(|l| timestamp(l.listed_at)),
            member_count,
            blockers: blockers.iter().map(|b| b.code().to_owned()).collect(),
        }
    }
}

/// One server card in Discover.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveryGuildDto {
    pub id: Id,
    pub name: String,
    pub icon: Option<String>,
    pub description: Option<String>,
    pub category: String,
    pub tags: Vec<String>,
    pub approximate_member_count: u64,
    pub approximate_presence_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveryPageDto {
    pub guilds: Vec<DiscoveryGuildDto>,
    pub total: u64,
    pub offset: u32,
    pub limit: u32,
}

/// `GET /discovery/categories`.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveryCategoriesDto {
    pub categories: Vec<&'static str>,
}

// --- guild templates -------------------------------------------------------

/// A server template (Discord guild template shape).
#[derive(Debug, Clone, Serialize)]
pub struct GuildTemplateDto {
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub usage_count: u32,
    pub creator_id: Option<Id>,
    pub creator: Option<UserDto>,
    pub created_at: String,
    pub updated_at: String,
    pub source_guild_id: Option<Id>,
    pub serialized_source_guild: Value,
    pub is_dirty: bool,
}

impl GuildTemplateDto {
    pub fn new(
        template: &domain::template::GuildTemplate,
        creator: Option<&User>,
        is_dirty: bool,
    ) -> Self {
        Self {
            code: template.code.clone(),
            name: template.name.clone(),
            description: template.description.clone(),
            usage_count: template.usage_count,
            creator_id: template.creator_id.map(Id),
            creator: creator.map(UserDto::public),
            created_at: timestamp(template.created_at),
            updated_at: timestamp(template.updated_at),
            source_guild_id: template.source_guild_id.map(Id),
            serialized_source_guild: template.source.to_json(),
            is_dirty,
        }
    }
}
