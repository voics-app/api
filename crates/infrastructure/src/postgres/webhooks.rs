//! Webhook reads.

use async_trait::async_trait;
use sqlx::FromRow;

use application::ports::WebhookRepository;
use application::{AppError, AppResult};
use domain::application::{FollowedChannel, Webhook, WebhookKind};
use domain::Snowflake;

use super::convert::{sf, sf_opt};
use super::error::db;
use super::PgStore;

pub(super) const WEBHOOK_COLUMNS: &str =
    "id, kind, guild_id, channel_id, creator_id, name, avatar, application_id, token_hash";

#[derive(FromRow)]
pub(super) struct WebhookRow {
    id: i64,
    kind: i16,
    guild_id: i64,
    channel_id: i64,
    creator_id: Option<i64>,
    name: String,
    avatar: Option<String>,
    application_id: Option<i64>,
    token_hash: String,
}

impl TryFrom<WebhookRow> for Webhook {
    type Error = AppError;

    fn try_from(row: WebhookRow) -> Result<Self, Self::Error> {
        let kind = WebhookKind::from_code(row.kind as u8)
            .ok_or_else(|| AppError::internal(format!("unknown webhook kind {}", row.kind)))?;
        Ok(Webhook {
            id: sf(row.id),
            kind,
            guild_id: sf(row.guild_id),
            channel_id: sf(row.channel_id),
            creator_id: sf_opt(row.creator_id),
            name: row.name,
            avatar: row.avatar,
            application_id: sf_opt(row.application_id),
            token_hash: row.token_hash,
        })
    }
}

#[async_trait]
impl WebhookRepository for PgStore {
    async fn list_followed_for_guild(
        &self,
        guild_id: Snowflake,
    ) -> AppResult<Vec<FollowedChannel>> {
        let rows: Vec<(i64, i64, i64)> = sqlx::query_as(
            "SELECT f.source_channel_id, f.target_channel_id, f.webhook_id FROM announcement_followers f JOIN webhooks w ON w.id = f.webhook_id WHERE w.guild_id = $1 ORDER BY f.source_channel_id, f.target_channel_id"
        ).bind(guild_id.as_i64()).fetch_all(&self.pool).await.map_err(db)?;
        Ok(rows
            .into_iter()
            .map(|(source, target, webhook)| FollowedChannel {
                source_channel_id: sf(source),
                target_channel_id: sf(target),
                webhook_id: sf(webhook),
            })
            .collect())
    }
    async fn find(&self, id: Snowflake) -> AppResult<Option<Webhook>> {
        let sql = format!("SELECT {WEBHOOK_COLUMNS} FROM webhooks WHERE id = $1");
        let row: Option<WebhookRow> = sqlx::query_as(&sql)
            .bind(id.as_i64())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        row.map(Webhook::try_from).transpose()
    }

    async fn list_for_guild(&self, guild_id: Snowflake) -> AppResult<Vec<Webhook>> {
        let sql = format!("SELECT {WEBHOOK_COLUMNS} FROM webhooks WHERE guild_id = $1 ORDER BY id");
        let rows: Vec<WebhookRow> = sqlx::query_as(&sql)
            .bind(guild_id.as_i64())
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        rows.into_iter().map(Webhook::try_from).collect()
    }
}
