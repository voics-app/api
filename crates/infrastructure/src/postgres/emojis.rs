//! Emoji reads, with the restricted role list loaded.

use async_trait::async_trait;
use sqlx::FromRow;

use application::ports::EmojiRepository;
use application::AppResult;
use domain::emoji::Emoji;
use domain::Snowflake;

use super::convert::{sf, sf_opt, sfs};
use super::error::db;
use super::PgStore;

const EMOJI_SELECT: &str =
    "SELECT e.id, e.guild_id, e.name, e.creator_id, e.animated, e.available, \
     e.require_colons, e.managed, \
     ARRAY(SELECT role_id FROM emoji_roles r WHERE r.emoji_id = e.id ORDER BY role_id) AS role_ids \
     FROM emojis e";

#[derive(FromRow)]
struct EmojiRow {
    id: i64,
    guild_id: i64,
    name: String,
    creator_id: Option<i64>,
    animated: bool,
    available: bool,
    require_colons: bool,
    managed: bool,
    role_ids: Vec<i64>,
}

impl From<EmojiRow> for Emoji {
    fn from(row: EmojiRow) -> Self {
        Emoji {
            id: sf(row.id),
            guild_id: sf(row.guild_id),
            name: row.name,
            creator_id: sf_opt(row.creator_id),
            role_ids: sfs(row.role_ids),
            animated: row.animated,
            available: row.available,
            require_colons: row.require_colons,
            managed: row.managed,
        }
    }
}

#[async_trait]
impl EmojiRepository for PgStore {
    async fn list(&self, guild_id: Snowflake) -> AppResult<Vec<Emoji>> {
        let sql = format!("{EMOJI_SELECT} WHERE e.guild_id = $1 ORDER BY e.id");
        let rows: Vec<EmojiRow> = sqlx::query_as(&sql)
            .bind(guild_id.as_i64())
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        Ok(rows.into_iter().map(Emoji::from).collect())
    }

    async fn find(&self, guild_id: Snowflake, emoji_id: Snowflake) -> AppResult<Option<Emoji>> {
        let sql = format!("{EMOJI_SELECT} WHERE e.guild_id = $1 AND e.id = $2");
        let row: Option<EmojiRow> = sqlx::query_as(&sql)
            .bind(guild_id.as_i64())
            .bind(emoji_id.as_i64())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        Ok(row.map(Emoji::from))
    }
}
