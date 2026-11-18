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
    /// Discord `system_channel_flags` bits (see [`SYSTEM_CHANNEL_FLAGS_MASK`]).
    pub system_channel_flags: u32,
    pub banner: Option<String>,
    pub splash: Option<String>,
    /// Only administrators may prune inactive members while this is set.
    pub prune_admin_only: bool,
    pub audit_policy: crate::audit_policy::AuditPolicy,
    pub community: Option<CommunityChannels>,
    pub created_at: DateTime<Utc>,
}

/// A configured Community always has both channels; one channel may serve both purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommunityChannels {
    pub rules_channel_id: Snowflake,
    pub public_updates_channel_id: Snowflake,
}

impl CommunityChannels {
    pub fn contains(self, id: Snowflake) -> bool {
        self.rules_channel_id == id || self.public_updates_channel_id == id
    }
}

/// `system_channel_flags` bit: do not post a message when a member joins.
pub const SUPPRESS_JOIN_NOTIFICATIONS: u32 = 1 << 0;
/// Every defined `system_channel_flags` bit (Discord bits 0 to 5).
pub const SYSTEM_CHANNEL_FLAGS_MASK: u32 = (1 << 6) - 1;

impl Guild {
    /// Whether a member joining should be announced in the system channel.
    pub const fn announces_joins(&self) -> bool {
        self.system_channel_id.is_some()
            && self.system_channel_flags & SUPPRESS_JOIN_NOTIFICATIONS == 0
    }

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

    /// A user-editable copy named "<name> (copy)" (cut to the 100 character limit) that
    /// takes this role's position; the caller shifts this role and those above it up by one.
    /// The copy is never managed, so it can be edited and deleted like any custom role.
    pub fn duplicate(&self, id: Snowflake) -> Self {
        const SUFFIX: &str = " (copy)";
        let stem: String = self
            .name
            .trim()
            .chars()
            .take(crate::validation::ROLE_NAME_MAX - SUFFIX.chars().count())
            .collect();
        Self {
            id,
            name: format!("{}{SUFFIX}", stem.trim_end()),
            managed: false,
            bot_id: None,
            ..self.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub guild_id: Snowflake,
    pub user_id: Snowflake,
    pub nick: Option<String>,
    pub avatar: Option<String>,
    pub banner: Option<String>,
    pub bio: Option<String>,
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
            banner: None,
            bio: None,
            role_ids: Vec::new(),
            joined_at,
            deaf: false,
            mute: false,
            communication_disabled_until: None,
            pending: false,
        }
    }

    pub fn is_timed_out(&self, now: DateTime<Utc>) -> bool {
        self.communication_disabled_until
            .is_some_and(|until| until > now)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn role(name: &str) -> Role {
        Role {
            id: Snowflake::new(5),
            guild_id: Snowflake::new(1),
            name: name.to_owned(),
            color: 0x12_34_56,
            hoist: true,
            icon: Some("hash".into()),
            unicode_emoji: Some("x".into()),
            position: 4,
            permissions: Permissions::from_bits_truncate(0b1010),
            managed: true,
            mentionable: true,
            bot_id: Some(Snowflake::new(9)),
        }
    }

    #[test]
    fn duplicate_copies_settings_under_a_new_id() {
        let source = role("Moderators");
        let copy = source.duplicate(Snowflake::new(6));
        assert_eq!(copy.id, Snowflake::new(6));
        assert_eq!(copy.name, "Moderators (copy)");
        assert_eq!(copy.position, source.position);
        assert_eq!(copy.permissions, source.permissions);
        assert_eq!(copy.color, source.color);
        assert_eq!(copy.hoist, source.hoist);
        assert_eq!(copy.mentionable, source.mentionable);
        assert_eq!(copy.icon, source.icon);
        assert_eq!(copy.unicode_emoji, source.unicode_emoji);
        assert!(!copy.managed, "a copy is a custom role");
        assert_eq!(copy.bot_id, None);
    }

    #[test]
    fn duplicate_name_stays_within_the_role_name_limit() {
        let copy = role(&"a".repeat(100)).duplicate(Snowflake::new(6));
        assert_eq!(copy.name.chars().count(), 100);
        assert!(copy.name.ends_with(" (copy)"));
        assert!(crate::validation::role_name(&copy.name).is_ok());
    }
}
