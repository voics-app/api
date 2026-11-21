use crate::Snowflake;

/// A stage that is currently live in a guild stage channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageInstance {
    pub id: Snowflake,
    pub guild_id: Snowflake,
    pub channel_id: Snowflake,
    pub topic: String,
}
