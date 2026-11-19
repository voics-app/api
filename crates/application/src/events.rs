//! Domain events emitted by use cases. Infrastructure maps them onto the
//! P6 envelope and topics.

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use domain::application::Interaction;
use domain::audit::AuditLogEntry;
use domain::channel::Channel;
use domain::delivery_mode::DeliveryMode;
use domain::device::Device;
use domain::emoji::Emoji;
use domain::guild::{Guild, GuildSnapshot, MemberProfile, Role};
use domain::invite::Invite;
use domain::notification::GuildNotificationSettings;
use domain::permissions::Permissions;
use domain::relationship::RelationshipType;
use domain::user::{User, UserSettings};
use domain::Snowflake;

/// A DM or group DM with everything needed to render it.
#[derive(Debug, Clone, PartialEq)]
pub struct PrivateChannelView {
    pub channel: Channel,
    pub recipients: Vec<User>,
    pub delivery_mode: DeliveryMode,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DomainEvent {
    // discordy.guild.v1
    /// Sent to `recipients` only (a creator or a joining member).
    GuildCreate {
        snapshot: Box<GuildSnapshot>,
        recipients: Vec<Snowflake>,
    },
    GuildUpdate(Guild),
    /// `recipients: None` fans out to the whole guild.
    GuildDelete {
        guild_id: Snowflake,
        recipients: Option<Vec<Snowflake>>,
    },
    ChannelCreate(Channel),
    ChannelUpdate(Channel),
    ChannelDelete(Channel),
    RoleCreate(Role),
    RoleUpdate(Role),
    RoleDelete {
        guild_id: Snowflake,
        role_id: Snowflake,
    },
    MemberAdd(MemberProfile),
    MemberUpdate(MemberProfile),
    MemberRemove {
        guild_id: Snowflake,
        user: User,
    },
    /// Bus-only: asks the gateway to move `user_id`'s voice session to `channel_id`, or to
    /// disconnect it when `None`. The gateway ignores it when the user is not in voice.
    VoiceMemberMove {
        guild_id: Snowflake,
        user_id: Snowflake,
        channel_id: Option<Snowflake>,
    },
    BanAdd {
        guild_id: Snowflake,
        user: User,
    },
    BanRemove {
        guild_id: Snowflake,
        user: User,
    },
    /// Bus-only: asks the messages service to delete `user_id`'s messages created at or after
    /// `since_ms` in `channel_ids` (a ban with `delete_message_seconds`).
    BanMessagePurge {
        guild_id: Snowflake,
        user_id: Snowflake,
        channel_ids: Vec<Snowflake>,
        since_ms: u64,
        delete_message_seconds: u32,
    },
    EmojisUpdate {
        guild_id: Snowflake,
        emojis: Vec<Emoji>,
    },
    InviteCreate {
        invite: Invite,
        inviter: Option<User>,
    },
    InviteDelete {
        guild_id: Snowflake,
        channel_id: Snowflake,
        code: String,
    },
    ThreadCreate(Channel),
    ThreadUpdate(Channel),
    ThreadDelete(Channel),
    ThreadMembersUpdate {
        guild_id: Snowflake,
        thread_id: Snowflake,
        user_id: Snowflake,
        joined_at: Option<DateTime<Utc>>,
        member_count: usize,
        version: u64,
        member_ids: Vec<Snowflake>,
    },

    // discordy.user.v1
    UserUpdate(User),
    UserSettingsUpdate(UserSettings),
    /// The user's notification settings for one guild or for DMs; sent to that user only.
    UserGuildSettingsUpdate(GuildNotificationSettings),
    /// The user's private note about `target_id` changed (empty `note` = removed); owner only.
    UserNoteUpdate {
        owner_id: Snowflake,
        target_id: Snowflake,
        note: String,
    },
    RelationshipAdd {
        user_id: Snowflake,
        kind: RelationshipType,
        target: User,
        since: DateTime<Utc>,
    },
    RelationshipRemove {
        user_id: Snowflake,
        target_id: Snowflake,
        kind: RelationshipType,
    },
    /// One envelope per recipient, each excluding that recipient from `recipients`.
    PrivateChannelCreate(PrivateChannelView),
    PrivateChannelUpdate(PrivateChannelView),
    /// CHANNEL_CREATE for one member only (someone was added to an existing group DM).
    PrivateChannelCreateFor {
        view: PrivateChannelView,
        user_id: Snowflake,
    },
    /// Sent to `user_id` when they leave a group DM.
    PrivateChannelDelete {
        channel: Channel,
        user_id: Snowflake,
    },
    RecipientAdd {
        channel_id: Snowflake,
        user: User,
        notify: Vec<Snowflake>,
    },
    RecipientRemove {
        channel_id: Snowflake,
        user: User,
        notify: Vec<Snowflake>,
    },
    /// The user's full device list after a registration or removal.
    UserDevicesUpdate {
        user_id: Snowflake,
        devices: Vec<Device>,
        target_user_ids: Vec<Snowflake>,
    },

    /// Bus-only: login sessions were revoked. The gateway closes the live connections of
    /// `session_ids` and never dispatches it.
    SessionsRevoke {
        user_id: Snowflake,
        session_ids: Vec<Uuid>,
    },

    // discordy.message.v1
    TypingStart {
        channel_id: Snowflake,
        guild_id: Option<Snowflake>,
        /// Participants of a DM or group DM (empty for guild channels).
        recipient_ids: Vec<Snowflake>,
        user_id: Snowflake,
        timestamp: DateTime<Utc>,
        member: Option<Box<MemberProfile>>,
    },

    // discordy.interaction.v1
    InteractionCreate {
        application_id: Snowflake,
        bot_user_id: Snowflake,
        interaction: Interaction,
        /// The raw interaction token; only its hash is stored.
        token: String,
        /// The bot's permissions in the channel.
        app_permissions: Permissions,
        user: User,
        member: Option<Box<MemberProfile>>,
    },
    /// Sent back to the invoking user (modal or autocomplete result).
    InteractionUserNotice {
        application_id: Snowflake,
        user_id: Snowflake,
        event_type: &'static str,
        payload: Value,
    },

    // discordy.audit.v1
    AuditLogEntryCreate(AuditLogEntry),
}

/// Any channel as returned by the REST API.
#[derive(Debug, Clone, PartialEq)]
pub enum ChannelView {
    Guild(Channel),
    Private(PrivateChannelView),
}
