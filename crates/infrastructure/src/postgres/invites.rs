//! Invite reads.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::FromRow;

use application::ports::InviteRepository;
use application::AppResult;
use domain::invite::Invite;
use domain::Snowflake;

use super::convert::{sf, sf_opt};
use super::error::db;
use super::PgStore;

const INVITE_COLUMNS: &str =
    "code, guild_id, channel_id, inviter_id, max_age, max_uses, uses, temporary, created_at";

#[derive(FromRow)]
struct InviteRow {
    code: String,
    guild_id: i64,
    channel_id: i64,
    inviter_id: Option<i64>,
    max_age: i32,
    max_uses: i32,
    uses: i32,
    temporary: bool,
    created_at: DateTime<Utc>,
}

impl From<InviteRow> for Invite {
    fn from(row: InviteRow) -> Self {
        Invite {
            code: row.code,
            guild_id: sf(row.guild_id),
            channel_id: sf(row.channel_id),
            inviter_id: sf_opt(row.inviter_id),
            max_age: row.max_age as u32,
            max_uses: row.max_uses as u32,
            uses: row.uses as u32,
            temporary: row.temporary,
            created_at: row.created_at,
        }
    }
}

#[async_trait]
impl InviteRepository for PgStore {
    async fn vanity_for_guild(
        &self,
        guild_id: Snowflake,
    ) -> AppResult<Option<domain::vanity::VanityInvite>> {
        super::vanity::for_guild(self, guild_id).await
    }

    async fn vanity_by_code(&self, code: &str) -> AppResult<Option<domain::vanity::VanityInvite>> {
        super::vanity::by_code(self, code).await
    }

    async fn find(&self, code: &str) -> AppResult<Option<Invite>> {
        let sql = format!("SELECT {INVITE_COLUMNS} FROM invites WHERE code = $1");
        let row: Option<InviteRow> = sqlx::query_as(&sql)
            .bind(code)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        Ok(row.map(Invite::from))
    }

    async fn list_for_guild(&self, guild_id: Snowflake) -> AppResult<Vec<Invite>> {
        let sql = format!(
            "SELECT {INVITE_COLUMNS} FROM invites WHERE guild_id = $1 ORDER BY created_at DESC"
        );
        let rows: Vec<InviteRow> = sqlx::query_as(&sql)
            .bind(guild_id.as_i64())
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        Ok(rows.into_iter().map(Invite::from).collect())
    }
}
