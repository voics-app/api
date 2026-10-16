use chrono::{DateTime, Utc};

use crate::channel::Channel;
use crate::emoji::Emoji;
use crate::permissions::Permissions;
use crate::snowflake::Snowflake;
use crate::user::User;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guild {
    pub id: Snowflake,
    pub name: String,
    pub icon: Option<String>,
    pub description: Option<String>,
    pub owner_id: Snowflake,
    pub voice_region: Option<String>,
    pub verification_level: u8,
    pub default_message_notifications: u8,
    pub explicit_content_filter: u8,
    pub afk_channel_id: Option<Snowflake>,
    pub afk_timeout: u32,
    pub system_channel_id: Option<Snowflake>,
    pub created_at: DateTime<Utc>,
}

impl Guild {
    /// The @everyone role shares the guild's id.
    pub const fn everyone_role_id(&self) -> Snowflake {
        self.id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Role {
    pub id: Snowflake,
    pub guild_id: Snowflake,
    pub name: String,
    pub color: u32,
    pub hoist: bool,
    pub icon: Option<String>,
    pub unicode_emoji: Option<String>,
    pub position: i32,
    pub permissions: Permissions,
    pub managed: bool,
    pub mentionable: bool,
    /// Set for the managed role created when a bot joins.
    pub bot_id: Option<Snowflake>,
}

impl Role {
    pub fn is_everyone(&self) -> bool {
        self.id == self.guild_id
    }

    pub fn everyone(guild_id: Snowflake, permissions: Permissions) -> Self {
        Self {
            id: guild_id,
            guild_id,
            name: "@everyone".to_owned(),
            color: 0,
            hoist: false,
            icon: None,
            unicode_emoji: None,
            position: 0,
            permissions,
            managed: false,
            mentionable: false,
            bot_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub guild_id: Snowflake,
    pub user_id: Snowflake,
    pub nick: Option<String>,
    pub avatar: Option<String>,
    pub role_ids: Vec<Snowflake>,
    pub joined_at: DateTime<Utc>,
    pub deaf: bool,
    pub mute: bool,
    pub communication_disabled_until: Option<DateTime<Utc>>,
    pub pending: bool,
}

impl Member {
    pub fn new(guild_id: Snowflake, user_id: Snowflake, joined_at: DateTime<Utc>) -> Self {
        Self {
            guild_id,
            user_id,
            nick: None,
            avatar: None,
            role_ids: Vec::new(),
            joined_at,
            deaf: false,
            mute: false,
            communication_disabled_until: None,
            pending: false,
        }
    }

    pub fn is_timed_out(&self, now: DateTime<Utc>) -> bool {
        self.communication_disabled_until.is_some_and(|until| until > now)
    }
}

/// A member together with its user, as Discord's member object embeds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberProfile {
    pub member: Member,
    pub user: User,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ban {
    pub guild_id: Snowflake,
    pub user: User,
    pub reason: Option<String>,
}

/// Everything about a guild a gateway guild process needs (`GuildFull`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildSnapshot {
    pub guild: Guild,
    pub roles: Vec<Role>,
    pub channels: Vec<Channel>,
    pub members: Vec<MemberProfile>,
    pub emojis: Vec<Emoji>,
    pub member_count: u64,
}
