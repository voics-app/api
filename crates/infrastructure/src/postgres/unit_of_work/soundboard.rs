use super::super::error::db;
use application::{AppError, AppResult};
use domain::soundboard::{SoundboardSound, MAX_SOUNDS_PER_GUILD};
use domain::Snowflake;
use sqlx::PgConnection;

pub(super) async fn insert(conn: &mut PgConnection, sound: &SoundboardSound) -> AppResult<()> {
    // Serialize insertions for one guild so the limit cannot be raced.
    sqlx::query("SELECT id FROM guilds WHERE id=$1 FOR UPDATE")
        .bind(sound.guild_id.as_i64())
        .fetch_one(&mut *conn)
        .await
        .map_err(db)?;
    let (count,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM soundboard_sounds WHERE guild_id=$1")
            .bind(sound.guild_id.as_i64())
            .fetch_one(&mut *conn)
            .await
            .map_err(db)?;
    if count >= MAX_SOUNDS_PER_GUILD as i64 {
        return Err(AppError::BadRequest {
            code: 30000,
            message: "Maximum number of sounds reached".into(),
        });
    }
    sqlx::query("INSERT INTO soundboard_sounds (id,guild_id,creator_id,name,volume,audio) VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(sound.id.as_i64()).bind(sound.guild_id.as_i64()).bind(sound.creator_id.map(|id| id.as_i64())).bind(&sound.name).bind(sound.volume).bind(&sound.audio)
        .execute(&mut *conn).await.map_err(db)?;
    Ok(())
}
pub(super) async fn update(conn: &mut PgConnection, sound: &SoundboardSound) -> AppResult<()> {
    sqlx::query("UPDATE soundboard_sounds SET name=$3,volume=$4 WHERE guild_id=$1 AND id=$2")
        .bind(sound.guild_id.as_i64())
        .bind(sound.id.as_i64())
        .bind(&sound.name)
        .bind(sound.volume)
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    Ok(())
}
pub(super) async fn delete(
    conn: &mut PgConnection,
    guild_id: Snowflake,
    sound_id: Snowflake,
) -> AppResult<()> {
    sqlx::query("DELETE FROM soundboard_sounds WHERE guild_id=$1 AND id=$2")
        .bind(guild_id.as_i64())
        .bind(sound_id.as_i64())
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    Ok(())
}
