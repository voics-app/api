use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::snowflake::Snowflake;

/// Discord audit log event codes used by Discordy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuditAction {
    GuildUpdate,
    ChannelCreate,
    ChannelUpdate,
    ChannelDelete,
    ChannelOverwriteCreate,
    ChannelOverwriteUpdate,
    ChannelOverwriteDelete,
    MemberKick,
    MemberPrune,
    MemberBanAdd,
    MemberBanRemove,
    MemberUpdate,
    MemberRoleUpdate,
    MemberMove,
    MemberDisconnect,
    BotAdd,
    RoleCreate,
    RoleUpdate,
    RoleDelete,
    InviteCreate,
    InviteDelete,
    WebhookCreate,
    WebhookUpdate,
    WebhookDelete,
    EmojiCreate,
    EmojiUpdate,
    EmojiDelete,
    ThreadCreate,
    ThreadUpdate,
    ThreadDelete,
}

impl AuditAction {
    pub const fn code(self) -> u16 {
        use AuditAction::*;
        match self {
            GuildUpdate => 1,
            ChannelCreate => 10,
            ChannelUpdate => 11,
            ChannelDelete => 12,
            ChannelOverwriteCreate => 13,
            ChannelOverwriteUpdate => 14,
            ChannelOverwriteDelete => 15,
            MemberKick => 20,
            MemberPrune => 21,
            MemberBanAdd => 22,
            MemberBanRemove => 23,
            MemberUpdate => 24,
            MemberRoleUpdate => 25,
            MemberMove => 26,
            MemberDisconnect => 27,
            BotAdd => 28,
            RoleCreate => 30,
            RoleUpdate => 31,
            RoleDelete => 32,
            InviteCreate => 40,
            InviteDelete => 42,
            WebhookCreate => 50,
            WebhookUpdate => 51,
            WebhookDelete => 52,
            EmojiCreate => 60,
            EmojiUpdate => 61,
            EmojiDelete => 62,
            ThreadCreate => 110,
            ThreadUpdate => 111,
            ThreadDelete => 112,
        }
    }

    pub const ALL: [AuditAction; 30] = {
        use AuditAction::*;
        [
            GuildUpdate,
            ChannelCreate,
            ChannelUpdate,
            ChannelDelete,
            ChannelOverwriteCreate,
            ChannelOverwriteUpdate,
            ChannelOverwriteDelete,
            MemberKick,
            MemberPrune,
            MemberBanAdd,
            MemberBanRemove,
            MemberUpdate,
            MemberRoleUpdate,
            MemberMove,
            MemberDisconnect,
            BotAdd,
            RoleCreate,
            RoleUpdate,
            RoleDelete,
            InviteCreate,
            InviteDelete,
            WebhookCreate,
            WebhookUpdate,
            WebhookDelete,
            EmojiCreate,
            EmojiUpdate,
            EmojiDelete,
            ThreadCreate,
            ThreadUpdate,
            ThreadDelete,
        ]
    };

    pub fn from_code(code: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.code() == code)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuditChange {
    pub key: String,
    pub old_value: Option<Value>,
    pub new_value: Option<Value>,
}

impl AuditChange {
    pub fn new(key: &str, old_value: Option<Value>, new_value: Option<Value>) -> Self {
        Self {
            key: key.to_owned(),
            old_value,
            new_value,
        }
    }

    /// Only records the change when the value actually differs.
    pub fn if_changed(key: &str, old_value: Value, new_value: Value) -> Option<Self> {
        (old_value != new_value).then(|| Self::new(key, Some(old_value), Some(new_value)))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuditLogEntry {
    pub id: Snowflake,
    pub guild_id: Snowflake,
    pub user_id: Option<Snowflake>,
    pub target_id: Option<Snowflake>,
    pub action: AuditAction,
    pub changes: Vec<AuditChange>,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn codes_round_trip() {
        for action in AuditAction::ALL {
            assert_eq!(AuditAction::from_code(action.code()), Some(action));
        }
        assert_eq!(AuditAction::from_code(999), None);
        assert_eq!(AuditAction::MemberPrune.code(), 21);
        assert_eq!(AuditAction::MemberBanAdd.code(), 22);
        assert_eq!(AuditAction::MemberMove.code(), 26);
        assert_eq!(AuditAction::MemberDisconnect.code(), 27);
    }

    #[test]
    fn records_only_real_changes() {
        assert!(AuditChange::if_changed("name", json!("a"), json!("a")).is_none());
        let change = AuditChange::if_changed("name", json!("a"), json!("b")).unwrap();
        assert_eq!(change.new_value, Some(json!("b")));
    }
}
