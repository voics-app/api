use super::convert::{sf, sf_opt};
use super::error::db;
use super::PgStore;
use application::ports::SoundboardRepository;
use application::AppResult;
use async_trait::async_trait;
use domain::soundboard::SoundboardSound;
use domain::Snowflake;
use sqlx::FromRow;

#[derive(FromRow)]
struct SoundRow {
    id: i64,
    guild_id: i64,
    creator_id: Option<i64>,
    name: String,
    volume: f32,
    audio: Vec<u8>,
}
impl From<SoundRow> for SoundboardSound {
    fn from(row: SoundRow) -> Self {
        Self {
            id: sf(row.id),
            guild_id: sf(row.guild_id),
            creator_id: sf_opt(row.creator_id),
            name: row.name,
            volume: row.volume,
            audio: row.audio,
        }
    }
}
#[async_trait]
impl SoundboardRepository for PgStore {
    async fn list_sounds(&self, guild_id: Snowflake) -> AppResult<Vec<SoundboardSound>> {
        let rows: Vec<SoundRow> = sqlx::query_as("SELECT id,guild_id,creator_id,name,volume,audio FROM soundboard_sounds WHERE guild_id=$1 ORDER BY id")
            .bind(guild_id.as_i64()).fetch_all(&self.pool).await.map_err(db)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
    async fn find_sound(
        &self,
        guild_id: Snowflake,
        sound_id: Snowflake,
    ) -> AppResult<Option<SoundboardSound>> {
        let row: Option<SoundRow> = sqlx::query_as("SELECT id,guild_id,creator_id,name,volume,audio FROM soundboard_sounds WHERE guild_id=$1 AND id=$2")
            .bind(guild_id.as_i64()).bind(sound_id.as_i64()).fetch_optional(&self.pool).await.map_err(db)?;
        Ok(row.map(Into::into))
    }
}
