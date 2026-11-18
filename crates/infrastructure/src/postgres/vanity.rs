use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgConnection};

use application::{AppError, AppResult, Resource};
use domain::vanity::VanityInvite;
use domain::Snowflake;

use super::convert::sf;
use super::error::db;
use super::PgStore;

#[derive(FromRow)]
struct VanityRow {
    guild_id: i64,
    code: String,
    uses: i32,
    created_at: DateTime<Utc>,
}

impl From<VanityRow> for VanityInvite {
    fn from(row: VanityRow) -> Self {
        Self {
            guild_id: sf(row.guild_id),
            code: row.code,
            uses: row.uses as u32,
            created_at: row.created_at,
        }
    }
}

pub(super) async fn for_guild(
    store: &PgStore,
    guild_id: Snowflake,
) -> AppResult<Option<VanityInvite>> {
    sqlx::query_as::<_, VanityRow>(
        "SELECT guild_id, code, uses, created_at FROM guild_vanity_urls WHERE guild_id = $1",
    )
    .bind(guild_id.as_i64())
    .fetch_optional(&store.pool)
    .await
    .map_err(db)
    .map(|row| row.map(Into::into))
}

pub(super) async fn by_code(store: &PgStore, code: &str) -> AppResult<Option<VanityInvite>> {
    sqlx::query_as::<_, VanityRow>(
        "SELECT guild_id, code, uses, created_at FROM guild_vanity_urls WHERE code = $1",
    )
    .bind(code.to_ascii_lowercase())
    .fetch_optional(&store.pool)
    .await
    .map_err(db)
    .map(|row| row.map(Into::into))
}

pub(super) fn occupied_code() -> AppError {
    AppError::invalid(
        "code",
        "VANITY_CODE_UNAVAILABLE",
        "This invitation code is already in use.",
    )
}

/// Both ordinary and vanity creation take the same transaction lock. Checking
/// two independent tables without this lock would allow a concurrent collision.
pub(super) async fn lock_code(conn: &mut PgConnection, code: &str) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(lower($1), 0))")
        .bind(code)
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}

pub(super) async fn set(
    conn: &mut PgConnection,
    guild_id: Snowflake,
    invite: Option<&VanityInvite>,
) -> AppResult<()> {
    let Some(invite) = invite else {
        sqlx::query("DELETE FROM guild_vanity_urls WHERE guild_id = $1")
            .bind(guild_id.as_i64())
            .execute(conn)
            .await
            .map_err(db)?;
        return Ok(());
    };
    lock_code(conn, &invite.code).await?;
    let occupied: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM invites WHERE lower(code) = $1
            UNION ALL SELECT 1 FROM guild_vanity_urls WHERE code = $1 AND guild_id <> $2)",
    )
    .bind(&invite.code)
    .bind(guild_id.as_i64())
    .fetch_one(&mut *conn)
    .await
    .map_err(db)?;
    if occupied {
        return Err(occupied_code());
    }
    sqlx::query("INSERT INTO guild_vanity_urls (guild_id, code, uses, created_at) VALUES ($1, $2, 0, $3)
        ON CONFLICT (guild_id) DO UPDATE SET code = EXCLUDED.code, uses = 0, created_at = EXCLUDED.created_at")
        .bind(guild_id.as_i64()).bind(&invite.code).bind(invite.created_at)
        .execute(conn).await.map_err(db)?;
    Ok(())
}

pub(super) async fn consume(
    conn: &mut PgConnection,
    guild_id: Snowflake,
    code: &str,
) -> AppResult<()> {
    let result = sqlx::query(
        "UPDATE guild_vanity_urls SET uses = uses + 1 WHERE guild_id = $1 AND code = $2",
    )
    .bind(guild_id.as_i64())
    .bind(code)
    .execute(conn)
    .await
    .map_err(db)?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(Resource::Invite));
    }
    Ok(())
}

pub(super) async fn reserve_ordinary(conn: &mut PgConnection, code: &str) -> AppResult<()> {
    lock_code(conn, code).await?;
    let occupied: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM guild_vanity_urls WHERE code = lower($1))")
            .bind(code)
            .fetch_one(conn)
            .await
            .map_err(db)?;
    if occupied {
        return Err(occupied_code());
    }
    Ok(())
}
