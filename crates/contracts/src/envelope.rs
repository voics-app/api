//! The P6 event envelope and topic names.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::wire::{timestamp, Id};

pub const TOPIC_GUILD: &str = "discordy.guild.v1";
pub const TOPIC_MESSAGE: &str = "discordy.message.v1";
pub const TOPIC_USER: &str = "discordy.user.v1";
pub const TOPIC_INTERACTION: &str = "discordy.interaction.v1";
pub const TOPIC_AUDIT: &str = "discordy.audit.v1";

pub const ENVELOPE_VERSION: u8 = 1;

/// Topics api produces to; created by the compose init container and the cluster.
pub const API_TOPICS: [&str; 5] = [TOPIC_GUILD, TOPIC_MESSAGE, TOPIC_USER, TOPIC_INTERACTION, TOPIC_AUDIT];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Envelope {
    /// UUIDv7; consumers deduplicate on it.
    pub id: Uuid,
    #[serde(rename = "type")]
    pub kind: String,
    pub v: u8,
    pub occurred_at: String,
    pub guild_id: Option<Id>,
    pub channel_id: Option<Id>,
    pub target_user_ids: Option<Vec<Id>>,
    pub data: Value,
}

impl Envelope {
    pub fn new(
        id: Uuid,
        kind: &str,
        occurred_at: DateTime<Utc>,
        guild_id: Option<Id>,
        channel_id: Option<Id>,
        target_user_ids: Option<Vec<Id>>,
        data: Value,
    ) -> Self {
        Self {
            id,
            kind: kind.to_owned(),
            v: ENVELOPE_VERSION,
            occurred_at: timestamp(occurred_at),
            guild_id,
            channel_id,
            target_user_ids,
            data,
        }
    }
}
