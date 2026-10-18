//! Interaction reads.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::FromRow;

use application::ports::InteractionRepository;
use application::{AppError, AppResult};
use domain::application::{Interaction, InteractionKind, InteractionState};
use domain::Snowflake;

use super::convert::{sf, sf_opt};
use super::error::db;
use super::PgStore;

pub(super) const INTERACTION_COLUMNS: &str =
    "id, application_id, kind, guild_id, channel_id, user_id, token_hash, \
     data, state, original_message_id, source_message_id, created_at";

#[derive(FromRow)]
pub(super) struct InteractionRow {
    id: i64,
    application_id: i64,
    kind: i16,
    guild_id: Option<i64>,
    channel_id: i64,
    user_id: i64,
    token_hash: String,
    data: serde_json::Value,
    state: String,
    original_message_id: Option<i64>,
    source_message_id: Option<i64>,
    created_at: DateTime<Utc>,
}

impl TryFrom<InteractionRow> for Interaction {
    type Error = AppError;

    fn try_from(row: InteractionRow) -> Result<Self, Self::Error> {
        let kind = InteractionKind::from_code(row.kind as u8)
            .ok_or_else(|| AppError::internal(format!("unknown interaction kind {}", row.kind)))?;
        let state = InteractionState::parse(&row.state).ok_or_else(|| {
            AppError::internal(format!("unknown interaction state {}", row.state))
        })?;
        Ok(Interaction {
            id: sf(row.id),
            application_id: sf(row.application_id),
            kind,
            guild_id: sf_opt(row.guild_id),
            channel_id: sf(row.channel_id),
            user_id: sf(row.user_id),
            token_hash: row.token_hash,
            data: row.data,
            state,
            original_message_id: sf_opt(row.original_message_id),
            source_message_id: sf_opt(row.source_message_id),
            created_at: row.created_at,
        })
    }
}

#[async_trait]
impl InteractionRepository for PgStore {
    async fn find(&self, id: Snowflake) -> AppResult<Option<Interaction>> {
        let sql = format!("SELECT {INTERACTION_COLUMNS} FROM interactions WHERE id = $1");
        let row: Option<InteractionRow> = sqlx::query_as(&sql)
            .bind(id.as_i64())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        row.map(Interaction::try_from).transpose()
    }

    async fn find_by_token(
        &self,
        application_id: Snowflake,
        token_hash: &str,
    ) -> AppResult<Option<Interaction>> {
        let sql = format!("SELECT {INTERACTION_COLUMNS} FROM interactions WHERE application_id = $1 AND token_hash = $2");
        let row: Option<InteractionRow> = sqlx::query_as(&sql)
            .bind(application_id.as_i64())
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        row.map(Interaction::try_from).transpose()
    }
}
