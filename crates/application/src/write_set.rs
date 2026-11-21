//! A batch of state changes plus the events they produce, committed in one
//! transaction by [`crate::ports::UnitOfWork`] (transactional outbox).

use chrono::{DateTime, Utc};

use domain::account::AccountState;
use domain::application::{
    Application, ApplicationCommand, Interaction, InteractionState, Webhook,
};
use domain::audit::AuditLogEntry;
use domain::channel::Channel;
use domain::delivery_mode::DeliveryMode;
use domain::device::Device;
use domain::emoji::Emoji;
use domain::guild::{Guild, Member, Role};
use domain::invite::Invite;
use domain::notification::GuildNotificationSettings;
use domain::relationship::RelationshipType;
use domain::user::{Credentials, Suspension, User, UserSettings};
use domain::Snowflake;

use crate::events::DomainEvent;

/// Only explicitly supplied metadata. Credential and identity fields are deliberately absent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WebhookMetadataPatch {
    pub name: Option<String>,
    pub avatar: Option<Option<String>>,
    pub channel_id: Option<Snowflake>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mutation {
    InsertScheduledEvent(domain::scheduled_event::ScheduledEvent),
    UpdateScheduledEvent {
        event: domain::scheduled_event::ScheduledEvent,
        expected_revision: i64,
    },
    DeleteScheduledEvent {
        guild_id: Snowflake,
        event_id: Snowflake,
        expected_revision: i64,
    },
    SetScheduledEventInterest {
        guild_id: Snowflake,
        event_id: Snowflake,
        user_id: Snowflake,
        channel_id: Option<Snowflake>,
        interested: bool,
    },
    InsertUser {
        user: User,
        credentials: Option<Credentials>,
        settings: Box<UserSettings>,
    },
    UpdateUser(User),
    SetUserAppearance {
        user_id: Snowflake,
        patch: domain::profile::AppearancePatch,
    },
    SetMemberProfile {
        guild_id: Snowflake,
        user_id: Snowflake,
        patch: domain::profile::MemberProfilePatch,
    },
    /// Activates (`Some`) or removes (`None`) TOTP, clearing any pending
    /// secret; removal also deletes the backup codes.
    SetTotpSecret {
        user_id: Snowflake,
        secret: Option<String>,
    },
    /// Replaces the password hash.
    SetPasswordHash {
        user_id: Snowflake,
        hash: String,
    },
    /// Stores or clears the unconfirmed TOTP secret of an enrolment.
    SetPendingTotp {
        user_id: Snowflake,
        secret: Option<String>,
    },
    /// Replaces every backup code with these hashes.
    ReplaceBackupCodes {
        user_id: Snowflake,
        hashes: Vec<String>,
    },
    SetAccountState {
        user_id: Snowflake,
        state: AccountState,
    },
    /// Final step of account deletion: strips personal data and credentials.
    AnonymizeUser {
        user_id: Snowflake,
        username: String,
    },
    SuspendUser {
        user_id: Snowflake,
        suspension: Suspension,
    },
    UpsertSettings(UserSettings),
    UpsertGuildNotificationSettings(GuildNotificationSettings),
    /// `note: None` removes the note.
    SetUserNote {
        owner_id: Snowflake,
        target_id: Snowflake,
        note: Option<String>,
    },
    /// `kind: None` removes the row.
    SetRelationship {
        user_id: Snowflake,
        target_id: Snowflake,
        kind: Option<RelationshipType>,
        since: DateTime<Utc>,
    },

    InsertChannel(Channel),
    /// Replaces every column, overwrites and recipients included.
    UpdateChannel(Channel),
    SetVoiceStatus {
        channel_id: Snowflake,
        status: Option<String>,
    },
    DeleteChannel(Snowflake),
    SetThreadMember {
        thread_id: Snowflake,
        guild_id: Snowflake,
        user_id: Snowflake,
        joined_at: Option<DateTime<Utc>>,
    },
    /// `mode: None` clears the override.
    SetDeliveryOverride {
        user_id: Snowflake,
        channel_id: Snowflake,
        mode: Option<DeliveryMode>,
    },

    InsertGuild(Guild),
    ApplyOnboarding {
        guild_id: Snowflake,
        user_id: Snowflake,
        revision: i64,
        role_ids: Vec<Snowflake>,
    },
    CheckScreeningRevision {
        guild_id: Snowflake,
        revision: i64,
    },
    AcceptScreening {
        guild_id: Snowflake,
        user_id: Snowflake,
    },
    UpdateGuild(Guild),
    DeleteGuild(Snowflake),
    InsertRole(Role),
    UpdateRole(Role),
    /// Makes room for the managed bot role without rewriting role metadata or permissions.
    ShiftRolesForBot {
        guild_id: Snowflake,
    },
    DeleteRole {
        guild_id: Snowflake,
        role_id: Snowflake,
    },
    /// Inserts or replaces the member and its role list.
    UpsertMember(Member),
    /// Rejects concurrent joins instead of resetting an existing member's roles.
    InsertMember(Member),
    DeleteMember {
        guild_id: Snowflake,
        user_id: Snowflake,
    },
    InsertBan {
        guild_id: Snowflake,
        user_id: Snowflake,
        reason: Option<String>,
    },
    DeleteBan {
        guild_id: Snowflake,
        user_id: Snowflake,
    },

    InsertInvite(Invite),
    DeleteInvite(String),
    /// Increments uses; fails with a conflict when the invite is exhausted.
    ConsumeInvite(String),
    SetVanityInvite {
        guild_id: Snowflake,
        invite: Option<domain::vanity::VanityInvite>,
    },
    ConsumeVanityInvite {
        guild_id: Snowflake,
        code: String,
    },

    InsertEmoji(Emoji),
    UpdateEmoji(Emoji),
    DeleteEmoji {
        guild_id: Snowflake,
        emoji_id: Snowflake,
    },

    InsertAudit(AuditLogEntry),
    SetAuditPolicy {
        guild_id: Snowflake,
        policy: domain::audit_policy::AuditPolicy,
    },
    SetCommunity {
        guild_id: Snowflake,
        channels: Option<domain::guild::CommunityChannels>,
    },

    InsertApplication(Application),
    UpdateApplication(Application),
    /// Replaces the bot's token.
    SetBotToken {
        application_id: Snowflake,
        bot_user_id: Snowflake,
        token_hash: String,
    },
    UpsertCommand(ApplicationCommand),
    DeleteCommand {
        application_id: Snowflake,
        command_id: Snowflake,
    },
    /// Bulk overwrite of one scope (global when `guild_id` is `None`).
    ReplaceCommands {
        application_id: Snowflake,
        guild_id: Option<Snowflake>,
        commands: Vec<ApplicationCommand>,
    },

    InsertDevice(Device),
    DeleteDevice {
        user_id: Snowflake,
        device_id: Snowflake,
    },

    InsertInteraction(Interaction),
    UpdateInteraction {
        id: Snowflake,
        state: InteractionState,
        original_message_id: Option<Snowflake>,
    },

    InsertWebhook(Webhook),
    InsertAnnouncementFollower {
        source_channel_id: Snowflake,
        target_channel_id: Snowflake,
        webhook_id: Snowflake,
    },
    EnqueueAnnouncement {
        source_channel_id: Snowflake,
        source_message_id: Snowflake,
        payload: serde_json::Value,
        /// (target channel, follower webhook, reserved destination message id).
        deliveries: Vec<(Snowflake, Snowflake, Snowflake)>,
    },
    MarkAnnouncementPublished {
        source_channel_id: Snowflake,
        source_message_id: Snowflake,
    },
    /// Serializes lifecycle changes and rejects actions based on a replaced installation.
    GuardApplicationInstallation {
        guild_id: Snowflake,
        application_id: Snowflake,
        bot_user_id: Snowflake,
        expected_generation: Option<i64>,
        expected_revision: Option<i64>,
    },
    RegisterApplicationInstallation {
        guild_id: Snowflake,
        application_id: Snowflake,
        bot_user_id: Snowflake,
        installed_at: DateTime<Utc>,
    },
    RecordBotConsent(domain::bot_authorization::BotConsent),
    RevokeBotConsent {
        id: Snowflake,
        user_id: Snowflake,
    },
    UpdateWebhook {
        id: Snowflake,
        patch: WebhookMetadataPatch,
    },
    /// Rotates only the incoming webhook credential; metadata is preserved.
    SetWebhookToken {
        id: Snowflake,
        token_hash: String,
    },
    DeleteWebhook(Snowflake),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WriteSet {
    pub mutations: Vec<Mutation>,
    pub events: Vec<DomainEvent>,
    /// A publication must still own this forum lease at commit time.
    pub forum_guard: Option<(Snowflake, String)>,
}

impl WriteSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn guard_forum_publication(&mut self, id: Snowflake, token: String) -> &mut Self {
        self.forum_guard = Some((id, token));
        self
    }

    pub fn mutate(&mut self, mutation: Mutation) -> &mut Self {
        self.mutations.push(mutation);
        self
    }

    pub fn emit(&mut self, event: DomainEvent) -> &mut Self {
        self.events.push(event);
        self
    }

    /// Persists an audit log entry and publishes AUDIT_LOG_ENTRY_CREATE.
    pub fn audit(&mut self, entry: AuditLogEntry) -> &mut Self {
        self.mutations.push(Mutation::InsertAudit(entry.clone()));
        self.events.push(DomainEvent::AuditLogEntryCreate(entry));
        self
    }

    pub fn is_empty(&self) -> bool {
        self.mutations.is_empty() && self.events.is_empty()
    }
}
