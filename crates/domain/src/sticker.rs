use crate::Snowflake;

pub const MAX_STICKERS_PER_GUILD: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sticker {
    pub id: Snowflake,
    pub guild_id: Snowflake,
    pub creator_id: Option<Snowflake>,
    pub name: String,
    pub tags: String,
    pub format_type: i16,
    pub available: bool,
}
