use super::convert::{sf, sf_opt};
use super::error::db;
use super::PgStore;
use application::ports::StickerRepository;
use application::AppResult;
use async_trait::async_trait;
use domain::sticker::Sticker;
use domain::Snowflake;
use sqlx::FromRow;

const SELECT: &str =
    "SELECT id, guild_id, creator_id, name, tags, format_type, available FROM stickers";

#[derive(FromRow)]
struct StickerRow {
    id: i64,
    guild_id: i64,
    creator_id: Option<i64>,
    name: String,
    tags: String,
    format_type: i16,
    available: bool,
}

impl From<StickerRow> for Sticker {
    fn from(row: StickerRow) -> Self {
        Self {
            id: sf(row.id),
            guild_id: sf(row.guild_id),
            creator_id: sf_opt(row.creator_id),
            name: row.name,
            tags: row.tags,
            format_type: row.format_type,
            available: row.available,
        }
    }
}

#[async_trait]
impl StickerRepository for PgStore {
    async fn list(&self, guild_id: Snowflake) -> AppResult<Vec<Sticker>> {
        let rows: Vec<StickerRow> = sqlx::query_as(&format!(
            "{SELECT} WHERE guild_id = $1 AND available ORDER BY id"
        ))
        .bind(guild_id.as_i64())
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows.into_iter().map(Sticker::from).collect())
    }

    async fn find(&self, guild_id: Snowflake, sticker_id: Snowflake) -> AppResult<Option<Sticker>> {
        let row: Option<StickerRow> = sqlx::query_as(&format!(
            "{SELECT} WHERE guild_id = $1 AND id = $2 AND available"
        ))
        .bind(guild_id.as_i64())
        .bind(sticker_id.as_i64())
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row.map(Sticker::from))
    }
}

pub(super) async fn committed(
    conn: &mut sqlx::PgConnection,
    guild_id: Snowflake,
) -> AppResult<Vec<Sticker>> {
    let rows: Vec<StickerRow> = sqlx::query_as(&format!(
        "{SELECT} WHERE guild_id = $1 AND available ORDER BY id"
    ))
    .bind(guild_id.as_i64())
    .fetch_all(conn)
    .await
    .map_err(db)?;
    Ok(rows.into_iter().map(Sticker::from).collect())
}
