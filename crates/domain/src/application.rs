//! Applications, bots, application commands, interactions and webhooks.

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

use crate::permissions::Permissions;
use crate::snowflake::Snowflake;
use crate::validation::FieldError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Application {
    pub id: Snowflake,
    pub name: String,
    pub description: String,
    pub icon: Option<String>,
    pub owner_id: Snowflake,
    pub bot_user_id: Snowflake,
    pub bot_public: bool,
    pub bot_require_code_grant: bool,
    /// Hex-encoded Ed25519 public key (Discord's `verify_key`).
    pub verify_key: String,
    pub flags: u64,
    pub interactions_endpoint_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Gateway intent flags on the application (Discord values).
pub mod application_flags {
    pub const GATEWAY_PRESENCE: u64 = 1 << 12;
    pub const GATEWAY_PRESENCE_LIMITED: u64 = 1 << 13;
    pub const GATEWAY_GUILD_MEMBERS: u64 = 1 << 14;
    pub const GATEWAY_GUILD_MEMBERS_LIMITED: u64 = 1 << 15;
    pub const GATEWAY_MESSAGE_CONTENT: u64 = 1 << 18;
    pub const GATEWAY_MESSAGE_CONTENT_LIMITED: u64 = 1 << 19;
    pub const USER_EDITABLE: u64 =
        GATEWAY_PRESENCE_LIMITED | GATEWAY_GUILD_MEMBERS_LIMITED | GATEWAY_MESSAGE_CONTENT_LIMITED;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandKind {
    ChatInput,
    User,
    Message,
}

impl CommandKind {
    pub const fn code(self) -> u8 {
        match self {
            CommandKind::ChatInput => 1,
            CommandKind::User => 2,
            CommandKind::Message => 3,
        }
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => CommandKind::ChatInput,
            2 => CommandKind::User,
            3 => CommandKind::Message,
            _ => return None,
        })
    }
}

pub const MAX_COMMANDS_PER_SCOPE: usize = 100;

#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationCommand {
    pub id: Snowflake,
    pub application_id: Snowflake,
    pub guild_id: Option<Snowflake>,
    pub name: String,
    pub description: String,
    pub kind: CommandKind,
    /// Discord's recursive option array, stored verbatim.
    pub options: Value,
    pub default_member_permissions: Option<Permissions>,
    pub dm_permission: bool,
    pub nsfw: bool,
    pub version: Snowflake,
}

/// Discord's naming rules: chat-input names are lowercase `[-_\p{L}\p{N}]{1,32}`
/// with a 1–100 character description; context-menu commands have 1–32
/// character names and an empty description.
pub fn validate_command(
    kind: CommandKind,
    name: &str,
    description: &str,
) -> Result<(), FieldError> {
    let name_len = name.chars().count();
    let bad = |field: &str, message: &str| FieldError {
        field: field.to_owned(),
        code: "APPLICATION_COMMAND_INVALID",
        message: message.to_owned(),
    };
    if !(1..=32).contains(&name_len) {
        return Err(bad(
            "name",
            "Command names must be between 1 and 32 characters.",
        ));
    }
    match kind {
        CommandKind::ChatInput => {
            let valid_chars = name
                .chars()
                .all(|c| c == '-' || c == '_' || (c.is_alphanumeric() && !c.is_uppercase()));
            if !valid_chars {
                return Err(bad(
                    "name",
                    "Chat input command names must be lowercase letters, numbers, - or _.",
                ));
            }
            if !(1..=100).contains(&description.chars().count()) {
                return Err(bad(
                    "description",
                    "Description must be between 1 and 100 characters.",
                ));
            }
        }
        CommandKind::User | CommandKind::Message => {
            if !description.is_empty() {
                return Err(bad(
                    "description",
                    "Context menu commands cannot have a description.",
                ));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionKind {
    Ping,
    ApplicationCommand,
    MessageComponent,
    Autocomplete,
    ModalSubmit,
}

impl InteractionKind {
    pub const fn code(self) -> u8 {
        match self {
            InteractionKind::Ping => 1,
            InteractionKind::ApplicationCommand => 2,
            InteractionKind::MessageComponent => 3,
            InteractionKind::Autocomplete => 4,
            InteractionKind::ModalSubmit => 5,
        }
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => InteractionKind::Ping,
            2 => InteractionKind::ApplicationCommand,
            3 => InteractionKind::MessageComponent,
            4 => InteractionKind::Autocomplete,
            5 => InteractionKind::ModalSubmit,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionState {
    /// Waiting for the initial callback.
    Pending,
    /// Acknowledged with a deferred response.
    Deferred,
    /// Answered.
    Responded,
}

impl InteractionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            InteractionState::Pending => "pending",
            InteractionState::Deferred => "deferred",
            InteractionState::Responded => "responded",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(InteractionState::Pending),
            "deferred" => Some(InteractionState::Deferred),
            "responded" => Some(InteractionState::Responded),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackKind {
    Pong,
    ChannelMessageWithSource,
    DeferredChannelMessageWithSource,
    DeferredUpdateMessage,
    UpdateMessage,
    AutocompleteResult,
    Modal,
}

impl CallbackKind {
    pub const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => CallbackKind::Pong,
            4 => CallbackKind::ChannelMessageWithSource,
            5 => CallbackKind::DeferredChannelMessageWithSource,
            6 => CallbackKind::DeferredUpdateMessage,
            7 => CallbackKind::UpdateMessage,
            8 => CallbackKind::AutocompleteResult,
            9 => CallbackKind::Modal,
            _ => return None,
        })
    }

    /// State the interaction moves to after this callback.
    pub const fn resulting_state(self) -> InteractionState {
        match self {
            CallbackKind::DeferredChannelMessageWithSource
            | CallbackKind::DeferredUpdateMessage => InteractionState::Deferred,
            _ => InteractionState::Responded,
        }
    }
}

/// Interaction tokens stay valid for follow-ups for 15 minutes.
pub const INTERACTION_TOKEN_TTL_MINUTES: i64 = 15;

#[derive(Debug, Clone, PartialEq)]
pub struct Interaction {
    pub id: Snowflake,
    pub application_id: Snowflake,
    pub kind: InteractionKind,
    pub guild_id: Option<Snowflake>,
    pub channel_id: Snowflake,
    pub user_id: Snowflake,
    pub token_hash: String,
    pub data: Value,
    pub state: InteractionState,
    pub original_message_id: Option<Snowflake>,
    /// Message the component was attached to (component interactions).
    pub source_message_id: Option<Snowflake>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InteractionError {
    #[error("interaction has already been acknowledged")]
    AlreadyAcknowledged,
    #[error("interaction token has expired")]
    Expired,
    #[error("this callback type is not valid for this interaction")]
    InvalidCallback,
    #[error("interaction has not been acknowledged yet")]
    NotAcknowledged,
}

impl Interaction {
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.created_at + Duration::minutes(INTERACTION_TOKEN_TTL_MINUTES)
    }

    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at()
    }

    /// Checks whether `callback` may be the initial response.
    pub fn accept_callback(
        &self,
        callback: CallbackKind,
        now: DateTime<Utc>,
    ) -> Result<InteractionState, InteractionError> {
        if self.is_expired(now) {
            return Err(InteractionError::Expired);
        }
        if self.state != InteractionState::Pending {
            return Err(InteractionError::AlreadyAcknowledged);
        }
        use CallbackKind::*;
        use InteractionKind::*;
        let allowed = match callback {
            Pong => self.kind == Ping,
            ChannelMessageWithSource | DeferredChannelMessageWithSource => {
                matches!(
                    self.kind,
                    ApplicationCommand | MessageComponent | ModalSubmit
                )
            }
            DeferredUpdateMessage | UpdateMessage => {
                matches!(self.kind, MessageComponent | ModalSubmit)
            }
            AutocompleteResult => self.kind == Autocomplete,
            Modal => matches!(self.kind, ApplicationCommand | MessageComponent),
        };
        if !allowed {
            return Err(InteractionError::InvalidCallback);
        }
        Ok(callback.resulting_state())
    }

    /// Follow-ups and `@original` edits need an acknowledged, live token.
    pub fn ensure_followup_allowed(&self, now: DateTime<Utc>) -> Result<(), InteractionError> {
        if self.is_expired(now) {
            return Err(InteractionError::Expired);
        }
        if self.state == InteractionState::Pending {
            return Err(InteractionError::NotAcknowledged);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookKind {
    Incoming,
    ChannelFollower,
    Application,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FollowedChannel {
    pub source_channel_id: Snowflake,
    pub target_channel_id: Snowflake,
    pub webhook_id: Snowflake,
}

impl WebhookKind {
    pub const fn code(self) -> u8 {
        match self {
            WebhookKind::Incoming => 1,
            WebhookKind::ChannelFollower => 2,
            WebhookKind::Application => 3,
        }
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(WebhookKind::Incoming),
            2 => Some(WebhookKind::ChannelFollower),
            3 => Some(WebhookKind::Application),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Webhook {
    pub id: Snowflake,
    pub kind: WebhookKind,
    pub guild_id: Snowflake,
    pub channel_id: Snowflake,
    pub creator_id: Option<Snowflake>,
    pub name: String,
    pub avatar: Option<String>,
    pub application_id: Option<Snowflake>,
    pub token_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_validation_table() {
        let cases = [
            (CommandKind::ChatInput, "ping", "Replies with pong", true),
            (CommandKind::ChatInput, "über-cmd_1", "ok", true),
            (CommandKind::ChatInput, "Ping", "upper", false),
            (CommandKind::ChatInput, "has space", "x", false),
            (CommandKind::ChatInput, "ping", "", false),
            (CommandKind::ChatInput, "", "x", false),
            (CommandKind::ChatInput, &"a".repeat(33), "x", false),
            (CommandKind::User, "Report User", "", true),
            (CommandKind::Message, "Bookmark", "has description", false),
        ];
        for (kind, name, description, valid) in cases {
            assert_eq!(
                validate_command(kind, name, description).is_ok(),
                valid,
                "{kind:?} {name:?}"
            );
        }
    }

    fn interaction(
        kind: InteractionKind,
        state: InteractionState,
        age_minutes: i64,
    ) -> Interaction {
        Interaction {
            id: Snowflake::new(1),
            application_id: Snowflake::new(2),
            kind,
            guild_id: None,
            channel_id: Snowflake::new(3),
            user_id: Snowflake::new(4),
            token_hash: "h".into(),
            data: Value::Null,
            state,
            original_message_id: None,
            source_message_id: None,
            created_at: Utc::now() - Duration::minutes(age_minutes),
        }
    }

    #[test]
    fn callback_acceptance_table() {
        use CallbackKind as C;
        use InteractionError as Err_;
        use InteractionKind as K;
        use InteractionState as S;
        let now = Utc::now();
        let cases = [
            (
                K::ApplicationCommand,
                S::Pending,
                0,
                C::ChannelMessageWithSource,
                Ok(S::Responded),
            ),
            (
                K::ApplicationCommand,
                S::Pending,
                0,
                C::DeferredChannelMessageWithSource,
                Ok(S::Deferred),
            ),
            (
                K::ApplicationCommand,
                S::Pending,
                0,
                C::Modal,
                Ok(S::Responded),
            ),
            (
                K::ApplicationCommand,
                S::Pending,
                0,
                C::UpdateMessage,
                Err(Err_::InvalidCallback),
            ),
            (
                K::ApplicationCommand,
                S::Pending,
                0,
                C::Pong,
                Err(Err_::InvalidCallback),
            ),
            (
                K::MessageComponent,
                S::Pending,
                0,
                C::UpdateMessage,
                Ok(S::Responded),
            ),
            (
                K::MessageComponent,
                S::Pending,
                0,
                C::DeferredUpdateMessage,
                Ok(S::Deferred),
            ),
            (
                K::Autocomplete,
                S::Pending,
                0,
                C::AutocompleteResult,
                Ok(S::Responded),
            ),
            (
                K::Autocomplete,
                S::Pending,
                0,
                C::ChannelMessageWithSource,
                Err(Err_::InvalidCallback),
            ),
            (
                K::ModalSubmit,
                S::Pending,
                0,
                C::Modal,
                Err(Err_::InvalidCallback),
            ),
            (K::Ping, S::Pending, 0, C::Pong, Ok(S::Responded)),
            (
                K::ApplicationCommand,
                S::Responded,
                0,
                C::ChannelMessageWithSource,
                Err(Err_::AlreadyAcknowledged),
            ),
            (
                K::ApplicationCommand,
                S::Deferred,
                0,
                C::ChannelMessageWithSource,
                Err(Err_::AlreadyAcknowledged),
            ),
            (
                K::ApplicationCommand,
                S::Pending,
                16,
                C::ChannelMessageWithSource,
                Err(Err_::Expired),
            ),
        ];
        for (kind, state, age, callback, expected) in cases {
            let i = interaction(kind, state, age);
            assert_eq!(
                i.accept_callback(callback, now),
                expected,
                "{kind:?}/{state:?}/{callback:?}"
            );
        }
    }

    #[test]
    fn followup_rules() {
        let now = Utc::now();
        let pending = interaction(
            InteractionKind::ApplicationCommand,
            InteractionState::Pending,
            0,
        );
        assert_eq!(
            pending.ensure_followup_allowed(now),
            Err(InteractionError::NotAcknowledged)
        );
        let deferred = interaction(
            InteractionKind::ApplicationCommand,
            InteractionState::Deferred,
            1,
        );
        assert!(deferred.ensure_followup_allowed(now).is_ok());
        let old = interaction(
            InteractionKind::ApplicationCommand,
            InteractionState::Responded,
            20,
        );
        assert_eq!(
            old.ensure_followup_allowed(now),
            Err(InteractionError::Expired)
        );
    }
}
