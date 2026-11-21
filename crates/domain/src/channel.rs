use chrono::{DateTime, Utc};

use crate::delivery_mode::DeliveryMode;
use crate::permissions::PermissionOverwrite;
use crate::snowflake::Snowflake;

/// Discord channel type values (P9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelType {
    GuildText,
    Dm,
    GuildVoice,
    GroupDm,
    GuildCategory,
    GuildAnnouncement,
    AnnouncementThread,
    PublicThread,
    PrivateThread,
    GuildStageVoice,
    GuildForum,
}

impl ChannelType {
    pub const fn code(self) -> u8 {
        match self {
            ChannelType::GuildText => 0,
            ChannelType::Dm => 1,
            ChannelType::GuildVoice => 2,
            ChannelType::GroupDm => 3,
            ChannelType::GuildCategory => 4,
            ChannelType::GuildAnnouncement => 5,
            ChannelType::AnnouncementThread => 10,
            ChannelType::PublicThread => 11,
            ChannelType::PrivateThread => 12,
            ChannelType::GuildStageVoice => 13,
            ChannelType::GuildForum => 15,
        }
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => ChannelType::GuildText,
            1 => ChannelType::Dm,
            2 => ChannelType::GuildVoice,
            3 => ChannelType::GroupDm,
            4 => ChannelType::GuildCategory,
            5 => ChannelType::GuildAnnouncement,
            10 => ChannelType::AnnouncementThread,
            11 => ChannelType::PublicThread,
            12 => ChannelType::PrivateThread,
            13 => ChannelType::GuildStageVoice,
            15 => ChannelType::GuildForum,
            _ => return None,
        })
    }

    pub const fn is_private(self) -> bool {
        matches!(self, ChannelType::Dm | ChannelType::GroupDm)
    }

    pub const fn is_thread(self) -> bool {
        matches!(
            self,
            ChannelType::AnnouncementThread
                | ChannelType::PublicThread
                | ChannelType::PrivateThread
        )
    }

    pub const fn is_voice(self) -> bool {
        matches!(self, ChannelType::GuildVoice | ChannelType::GuildStageVoice)
    }

    /// Types that can be created through `POST /guilds/{id}/channels`.
    pub const fn is_creatable_in_guild(self) -> bool {
        matches!(
            self,
            ChannelType::GuildText
                | ChannelType::GuildVoice
                | ChannelType::GuildCategory
                | ChannelType::GuildAnnouncement
                | ChannelType::GuildStageVoice
                | ChannelType::GuildForum
        )
    }

    /// Names are lowercased and dashed for these types.
    pub const fn has_text_style_name(self) -> bool {
        matches!(
            self,
            ChannelType::GuildText | ChannelType::GuildAnnouncement | ChannelType::GuildForum
        )
    }

    /// Channels that can host threads.
    pub const fn supports_threads(self) -> bool {
        matches!(
            self,
            ChannelType::GuildText | ChannelType::GuildAnnouncement | ChannelType::GuildForum
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMetadata {
    pub archived: bool,
    pub auto_archive_duration: u32,
    pub archive_timestamp: DateTime<Utc>,
    pub locked: bool,
    pub invitable: bool,
    pub create_timestamp: DateTime<Utc>,
}

/// Allowed `auto_archive_duration` values, in minutes.
pub const AUTO_ARCHIVE_DURATIONS: [u32; 4] = [60, 1440, 4320, 10080];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub id: Snowflake,
    pub kind: ChannelType,
    pub guild_id: Option<Snowflake>,
    pub position: i32,
    pub name: Option<String>,
    pub topic: Option<String>,
    pub status: Option<String>,
    pub nsfw: bool,
    /// Voice channels only: whether members may send messages to the channel's chat.
    pub text_chat_enabled: bool,
    pub bitrate: Option<u32>,
    pub user_limit: Option<u32>,
    pub rate_limit_per_user: u32,
    pub parent_id: Option<Snowflake>,
    pub owner_id: Option<Snowflake>,
    pub icon: Option<String>,
    pub recipient_ids: Vec<Snowflake>,
    /// Guild users who joined this thread. Never included in public channel DTOs.
    pub thread_member_ids: Vec<Snowflake>,
    pub thread_membership_version: u64,
    pub overwrites: Vec<PermissionOverwrite>,
    /// Configured mode; only meaningful for guild channels.
    pub delivery_mode: DeliveryMode,
    pub voice_region: Option<String>,
    pub thread_metadata: Option<ThreadMetadata>,
    pub forum_settings: serde_json::Value,
    pub applied_tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl Channel {
    /// A blank channel of the given type; callers fill in the rest.
    pub fn new(
        id: Snowflake,
        kind: ChannelType,
        guild_id: Option<Snowflake>,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            kind,
            guild_id,
            position: 0,
            name: None,
            topic: None,
            status: None,
            nsfw: false,
            text_chat_enabled: true,
            bitrate: kind.is_voice().then_some(64_000),
            user_limit: kind.is_voice().then_some(0),
            rate_limit_per_user: 0,
            parent_id: None,
            owner_id: None,
            icon: None,
            recipient_ids: Vec::new(),
            thread_member_ids: Vec::new(),
            thread_membership_version: 0,
            overwrites: Vec::new(),
            delivery_mode: DeliveryMode::Standard,
            voice_region: None,
            thread_metadata: None,
            forum_settings: serde_json::json!({}),
            applied_tags: Vec::new(),
            created_at,
        }
    }

    pub fn is_recipient(&self, user_id: Snowflake) -> bool {
        self.recipient_ids.contains(&user_id)
    }
}

/// Default cap on group DM participants (the api setting `GROUP_DM_MAX_PARTICIPANTS`).
pub const DEFAULT_MAX_GROUP_DM_PARTICIPANTS: usize = 25;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_type_codes_round_trip() {
        for code in [0u8, 1, 2, 3, 4, 5, 10, 11, 12, 13, 15] {
            assert_eq!(ChannelType::from_code(code).unwrap().code(), code);
        }
        assert!(ChannelType::from_code(6).is_none());
    }

    #[test]
    fn classification_table() {
        let cases = [
            (ChannelType::GuildText, false, false, false, true),
            (ChannelType::Dm, true, false, false, false),
            (ChannelType::GroupDm, true, false, false, false),
            (ChannelType::GuildVoice, false, false, true, true),
            (ChannelType::PublicThread, false, true, false, false),
            (ChannelType::GuildCategory, false, false, false, true),
            (ChannelType::GuildStageVoice, false, false, true, true),
        ];
        for (kind, private, thread, voice, creatable) in cases {
            assert_eq!(kind.is_private(), private, "{kind:?} private");
            assert_eq!(kind.is_thread(), thread, "{kind:?} thread");
            assert_eq!(kind.is_voice(), voice, "{kind:?} voice");
            assert_eq!(
                kind.is_creatable_in_guild(),
                creatable,
                "{kind:?} creatable"
            );
        }
    }

    #[test]
    fn voice_channels_get_default_bitrate() {
        let voice = Channel::new(
            Snowflake::new(1),
            ChannelType::GuildVoice,
            Some(Snowflake::new(2)),
            Utc::now(),
        );
        assert_eq!(voice.bitrate, Some(64_000));
        let text = Channel::new(
            Snowflake::new(1),
            ChannelType::GuildText,
            Some(Snowflake::new(2)),
            Utc::now(),
        );
        assert_eq!(text.bitrate, None);
    }
}
