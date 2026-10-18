use crate::snowflake::Snowflake;

pub const MAX_EMOJIS_PER_GUILD: usize = 100;
/// 256 KiB, Discord's limit.
pub const MAX_EMOJI_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Emoji {
    pub id: Snowflake,
    pub guild_id: Snowflake,
    pub name: String,
    pub creator_id: Option<Snowflake>,
    pub role_ids: Vec<Snowflake>,
    pub animated: bool,
    pub available: bool,
    pub require_colons: bool,
    pub managed: bool,
}
