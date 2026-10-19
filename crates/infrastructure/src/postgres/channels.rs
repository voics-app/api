//! Channel reads, with recipients, overwrites and thread metadata loaded.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};

use application::ports::ChannelRepository;
use application::AppResult;
use domain::channel::{Channel, ChannelType};
use domain::delivery_mode::DeliveryMode;
use domain::permissions::{OverwriteKind, PermissionOverwrite};
use domain::Snowflake;

use super::convert::{mode, permissions, sf, sf_opt, sfs, thread_metadata_from_json};
use super::error::db;
use super::PgStore;

pub(super) const CHANNEL_COLUMNS: &str = "id, kind, guild_id, position, name, topic, nsfw, text_chat_enabled, bitrate, user_limit, \
     rate_limit_per_user, parent_id, owner_id, icon, delivery_mode, voice_region, thread_metadata, created_at";

#[derive(FromRow)]
pub(super) struct ChannelRow {
    id: i64,
    kind: i16,
    guild_id: Option<i64>,
    position: i32,
    name: Option<String>,
    topic: Option<String>,
    nsfw: bool,
    text_chat_enabled: bool,
    bitrate: Option<i32>,
    user_limit: Option<i32>,
    rate_limit_per_user: i32,
    parent_id: Option<i64>,
    owner_id: Option<i64>,
    icon: Option<String>,
    delivery_mode: String,
    voice_region: Option<String>,
    thread_metadata: Option<serde_json::Value>,
    created_at: DateTime<Utc>,
}

/// Channel row with its recipients and overwrites attached.
pub(super) fn assemble(
    row: ChannelRow,
    recipients: &HashMap<i64, Vec<i64>>,
    overwrites: &HashMap<i64, Vec<OverwriteRow>>,
) -> Channel {
    let id = row.id;
    Channel {
        id: sf(id),
        kind: ChannelType::from_code(row.kind as u8).unwrap_or(ChannelType::GuildText),
        guild_id: sf_opt(row.guild_id),
        position: row.position,
        name: row.name,
        topic: row.topic,
        nsfw: row.nsfw,
        text_chat_enabled: row.text_chat_enabled,
        bitrate: row.bitrate.map(|v| v as u32),
        user_limit: row.user_limit.map(|v| v as u32),
        rate_limit_per_user: row.rate_limit_per_user as u32,
        parent_id: sf_opt(row.parent_id),
        owner_id: sf_opt(row.owner_id),
        icon: row.icon,
        recipient_ids: recipients
            .get(&id)
            .map(|ids| sfs(ids.clone()))
            .unwrap_or_default(),
        overwrites: overwrites
            .get(&id)
            .map(|rows| rows.iter().map(|o| o.clone().into()).collect())
            .unwrap_or_default(),
        delivery_mode: mode(&row.delivery_mode),
        voice_region: row.voice_region,
        thread_metadata: row.thread_metadata.and_then(thread_metadata_from_json),
        created_at: row.created_at,
    }
}

#[derive(FromRow, Clone)]
pub(super) struct OverwriteRow {
    channel_id: i64,
    target_id: i64,
    kind: i16,
    allow_bits: i64,
    deny_bits: i64,
}

impl From<OverwriteRow> for PermissionOverwrite {
    fn from(row: OverwriteRow) -> Self {
        PermissionOverwrite {
            id: sf(row.target_id),
            kind: OverwriteKind::from_code(row.kind as u8).unwrap_or(OverwriteKind::Role),
            allow: permissions(row.allow_bits),
            deny: permissions(row.deny_bits),
        }
    }
}

/// Loads the channels with the given rows' recipients and overwrites.
pub(super) async fn hydrate(
    pool: &PgPool,
    rows: Vec<ChannelRow>,
) -> Result<Vec<Channel>, sqlx::Error> {
    let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let recipient_rows: Vec<(i64, i64)> =
        sqlx::query_as("SELECT channel_id, user_id FROM channel_recipients WHERE channel_id = ANY($1) ORDER BY user_id")
            .bind(&ids)
            .fetch_all(pool)
            .await?;
    let overwrite_rows: Vec<OverwriteRow> = sqlx::query_as(
        "SELECT channel_id, target_id, kind, allow_bits, deny_bits FROM permission_overwrites \
         WHERE channel_id = ANY($1) ORDER BY channel_id, kind, target_id",
    )
    .bind(&ids)
    .fetch_all(pool)
    .await?;

    let mut recipients: HashMap<i64, Vec<i64>> = HashMap::new();
    for (channel_id, user_id) in recipient_rows {
        recipients.entry(channel_id).or_default().push(user_id);
    }
    let mut overwrites: HashMap<i64, Vec<OverwriteRow>> = HashMap::new();
    for row in overwrite_rows {
        overwrites.entry(row.channel_id).or_default().push(row);
    }
    Ok(rows
        .into_iter()
        .map(|row| assemble(row, &recipients, &overwrites))
        .collect())
}

impl PgStore {
    async fn channels_where(&self, filter: &str, bind: i64) -> AppResult<Vec<Channel>> {
        let sql =
            format!("SELECT {CHANNEL_COLUMNS} FROM channels WHERE {filter} ORDER BY position, id");
        let rows: Vec<ChannelRow> = sqlx::query_as(&sql)
            .bind(bind)
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        hydrate(&self.pool, rows).await.map_err(db)
    }
}

#[async_trait]
impl ChannelRepository for PgStore {
    async fn find(&self, id: Snowflake) -> AppResult<Option<Channel>> {
        let sql = format!("SELECT {CHANNEL_COLUMNS} FROM channels WHERE id = $1");
        let row: Option<ChannelRow> = sqlx::query_as(&sql)
            .bind(id.as_i64())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        match row {
            Some(row) => Ok(hydrate(&self.pool, vec![row]).await.map_err(db)?.pop()),
            None => Ok(None),
        }
    }

    async fn list_for_guild(&self, guild_id: Snowflake) -> AppResult<Vec<Channel>> {
        self.channels_where("guild_id = $1", guild_id.as_i64())
            .await
    }

    async fn list_private(&self, user_id: Snowflake) -> AppResult<Vec<Channel>> {
        let sql = format!(
            "SELECT c.{cols} FROM channels c JOIN channel_recipients r ON r.channel_id = c.id \
             WHERE r.user_id = $1 AND c.guild_id IS NULL ORDER BY c.id",
            cols = CHANNEL_COLUMNS.replace(", ", ", c.")
        );
        let rows: Vec<ChannelRow> = sqlx::query_as(&sql)
            .bind(user_id.as_i64())
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        hydrate(&self.pool, rows).await.map_err(db)
    }

    async fn find_dm(&self, a: Snowflake, b: Snowflake) -> AppResult<Option<Channel>> {
        let sql = format!(
            "SELECT c.{cols} FROM channels c \
             WHERE c.kind = 1 AND c.guild_id IS NULL \
               AND EXISTS (SELECT 1 FROM channel_recipients r WHERE r.channel_id = c.id AND r.user_id = $1) \
               AND EXISTS (SELECT 1 FROM channel_recipients r WHERE r.channel_id = c.id AND r.user_id = $2) \
             LIMIT 1",
            cols = CHANNEL_COLUMNS.replace(", ", ", c.")
        );
        let row: Option<ChannelRow> = sqlx::query_as(&sql)
            .bind(a.as_i64())
            .bind(b.as_i64())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        match row {
            Some(row) => Ok(hydrate(&self.pool, vec![row]).await.map_err(db)?.pop()),
            None => Ok(None),
        }
    }

    async fn delivery_overrides(
        &self,
        channel_id: Snowflake,
    ) -> AppResult<Vec<(Snowflake, DeliveryMode)>> {
        let rows: Vec<(i64, String)> =
            sqlx::query_as("SELECT user_id, mode FROM channel_delivery_overrides WHERE channel_id = $1 ORDER BY user_id")
                .bind(channel_id.as_i64())
                .fetch_all(&self.pool)
                .await
                .map_err(db)?;
        Ok(rows
            .into_iter()
            .map(|(user_id, raw)| (sf(user_id), mode(&raw)))
            .collect())
    }

    async fn count(&self) -> AppResult<u64> {
        let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM channels")
            .fetch_one(&self.pool)
            .await
            .map_err(db)?;
        Ok(count as u64)
    }
}
