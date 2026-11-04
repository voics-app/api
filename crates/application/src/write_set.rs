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

#[derive(Debug, Clone, PartialEq)]
pub enum Mutation {
    InsertUser {
        user: User,
        credentials: Option<Credentials>,
        settings: Box<UserSettings>,
    },
    UpdateUser(User),
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
    DeleteChannel(Snowflake),
    /// `mode: None` clears the override.
    SetDeliveryOverride {
        user_id: Snowflake,
        channel_id: Snowflake,
        mode: Option<DeliveryMode>,
    },

    InsertGuild(Guild),
    UpdateGuild(Guild),
    DeleteGuild(Snowflake),
    InsertRole(Role),
    UpdateRole(Role),
    DeleteRole {
        guild_id: Snowflake,
        role_id: Snowflake,
    },
    /// Inserts or replaces the member and its role list.
    UpsertMember(Member),
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

    InsertEmoji(Emoji),
    UpdateEmoji(Emoji),
    DeleteEmoji {
        guild_id: Snowflake,
        emoji_id: Snowflake,
    },

    InsertAudit(AuditLogEntry),

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
    UpdateWebhook(Webhook),
    DeleteWebhook(Snowflake),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WriteSet {
    pub mutations: Vec<Mutation>,
    pub events: Vec<DomainEvent>,
}

impl WriteSet {
    pub fn new() -> Self {
        Self::default()
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
