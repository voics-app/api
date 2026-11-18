//! Serialize Community configuration with channel edits on the parent guild row.
use super::error::db;
use application::{AppError, AppResult};
use domain::channel::{Channel, ChannelType};
use domain::delivery_mode::DeliveryMode;
use domain::guild::CommunityChannels;
use domain::Snowflake;
use sqlx::PgConnection;

fn invalid() -> AppError {
    AppError::invalid("community", "CHANNEL_INVALID", "Community requires non-age-restricted Standard text or announcement channels. Replace its configured channels or disable Community first.")
}

pub(super) async fn lock(
    conn: &mut PgConnection,
    guild_id: Snowflake,
) -> AppResult<Option<(Option<i64>, Option<i64>)>> {
    sqlx::query_as(
        "SELECT rules_channel_id, public_updates_channel_id FROM guilds WHERE id = $1 FOR UPDATE",
    )
    .bind(guild_id.as_i64())
    .fetch_optional(conn)
    .await
    .map_err(db)
}

pub(super) async fn set(
    conn: &mut PgConnection,
    guild_id: Snowflake,
    channels: Option<CommunityChannels>,
) -> AppResult<()> {
    lock(conn, guild_id).await?;
    if let Some(channels) = channels {
        for id in [
            channels.rules_channel_id,
            channels.public_updates_channel_id,
        ] {
            let valid: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM channels WHERE id = $1 AND guild_id = $2 AND kind IN (0, 5) AND NOT nsfw AND delivery_mode = 'standard')")
                .bind(id.as_i64()).bind(guild_id.as_i64()).fetch_one(&mut *conn).await.map_err(db)?;
            if !valid {
                return Err(invalid());
            }
        }
    }
    sqlx::query(
        "UPDATE guilds SET rules_channel_id = $2, public_updates_channel_id = $3 WHERE id = $1",
    )
    .bind(guild_id.as_i64())
    .bind(channels.map(|c| c.rules_channel_id.as_i64()))
    .bind(channels.map(|c| c.public_updates_channel_id.as_i64()))
    .execute(&mut *conn)
    .await
    .map_err(db)?;
    if channels.is_none() {
        sqlx::query("DELETE FROM guild_discovery WHERE guild_id = $1")
            .bind(guild_id.as_i64())
            .execute(conn)
            .await
            .map_err(db)?;
    }
    Ok(())
}

async fn configured(
    conn: &mut PgConnection,
    guild_id: Snowflake,
    channel_id: Snowflake,
) -> AppResult<bool> {
    Ok(lock(conn, guild_id).await?.is_some_and(|(rules, updates)| {
        rules == Some(channel_id.as_i64()) || updates == Some(channel_id.as_i64())
    }))
}

pub(super) async fn check_update(conn: &mut PgConnection, channel: &Channel) -> AppResult<()> {
    if let Some(guild_id) = channel.guild_id {
        let configured = configured(conn, guild_id, channel.id).await?;
        if configured
            && (channel.nsfw
                || channel.delivery_mode != DeliveryMode::Standard
                || !matches!(
                    channel.kind,
                    ChannelType::GuildText | ChannelType::GuildAnnouncement
                ))
        {
            return Err(invalid());
        }
    }
    Ok(())
}

pub(super) async fn check_delete(conn: &mut PgConnection, channel_id: Snowflake) -> AppResult<()> {
    let guild: Option<i64> = sqlx::query_scalar("SELECT guild_id FROM channels WHERE id = $1")
        .bind(channel_id.as_i64())
        .fetch_optional(&mut *conn)
        .await
        .map_err(db)?
        .flatten();
    if let Some(guild) = guild {
        if configured(conn, Snowflake::new(guild as u64), channel_id).await? {
            return Err(invalid());
        }
    }
    Ok(())
}
