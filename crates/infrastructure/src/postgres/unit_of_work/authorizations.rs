use crate::postgres::{convert::bits, error::db};
use application::{AppError, AppResult, Resource};
use chrono::{DateTime, Utc};
use domain::{bot_authorization::BotConsent, Snowflake};
use sqlx::PgConnection;

pub(super) async fn guard(
    conn: &mut PgConnection,
    guild: Snowflake,
    app: Snowflake,
    bot: Snowflake,
    expected: Option<i64>,
    expected_revision: Option<i64>,
) -> AppResult<()> {
    // One lock per guild also serializes managed-role insertion for different apps.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('bot-installation:' || $1::bigint::text, 0))")
        .bind(guild.as_i64()).execute(&mut *conn).await.map_err(db)?;
    // Lock membership first, matching ordinary kick/leave and their cascading FK deletes.
    let member: Option<i64> = sqlx::query_scalar(
        "SELECT user_id FROM members WHERE guild_id=$1 AND user_id=$2 FOR UPDATE",
    )
    .bind(guild.as_i64())
    .bind(bot.as_i64())
    .fetch_optional(&mut *conn)
    .await
    .map_err(db)?;
    let current: Option<i64> = sqlx::query_scalar("SELECT generation FROM guild_application_installations WHERE guild_id=$1 AND application_id=$2")
        .bind(guild.as_i64()).bind(app.as_i64()).fetch_optional(&mut *conn).await.map_err(db)?;
    let revision: Option<i64> = sqlx::query_scalar("SELECT last_generation FROM application_installation_versions WHERE guild_id=$1 AND application_id=$2")
        .bind(guild.as_i64()).bind(app.as_i64()).fetch_optional(&mut *conn).await.map_err(db)?;
    if current != expected
        || revision != expected_revision
        || (expected.is_none() && member.is_some())
    {
        return Err(AppError::Conflict {
            code: 50035,
            message: "The bot installation changed. Review it and try again.".into(),
        });
    }
    Ok(())
}

pub(super) async fn register(
    conn: &mut PgConnection,
    guild: Snowflake,
    app: Snowflake,
    bot: Snowflake,
    installed_at: DateTime<Utc>,
) -> AppResult<()> {
    let generation: i64 = sqlx::query_scalar("INSERT INTO guild_application_installations (guild_id, application_id, bot_user_id, installed_at) VALUES ($1,$2,$3,$4) RETURNING generation")
        .bind(guild.as_i64()).bind(app.as_i64()).bind(bot.as_i64()).bind(installed_at).fetch_one(&mut *conn).await.map_err(db)?;
    sqlx::query("INSERT INTO application_installation_versions (guild_id, application_id, last_generation) VALUES ($1,$2,$3) ON CONFLICT (guild_id, application_id) DO UPDATE SET last_generation=EXCLUDED.last_generation")
        .bind(guild.as_i64()).bind(app.as_i64()).bind(generation).execute(conn).await.map_err(db)?;
    Ok(())
}

pub(super) async fn consent(conn: &mut PgConnection, consent: &BotConsent) -> AppResult<()> {
    let result = sqlx::query("INSERT INTO application_authorizations (id,user_id,installation_generation,scopes,permissions,created_at,authorized_at) SELECT $1,$2,generation,$3,$4,$5,$5 FROM guild_application_installations WHERE guild_id=$6 AND application_id=$7 ON CONFLICT (user_id,installation_generation) DO UPDATE SET scopes=EXCLUDED.scopes, permissions=EXCLUDED.permissions, authorized_at=EXCLUDED.authorized_at")
        .bind(consent.id.as_i64()).bind(consent.user_id.as_i64()).bind(&consent.scopes).bind(bits(consent.permissions)).bind(consent.authorized_at)
        .bind(consent.guild_id.as_i64()).bind(consent.application_id.as_i64()).execute(conn).await.map_err(db)?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(Resource::Application));
    }
    Ok(())
}

pub(super) async fn revoke(
    conn: &mut PgConnection,
    id: Snowflake,
    user: Snowflake,
) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM application_authorizations WHERE id=$1 AND user_id=$2")
        .bind(id.as_i64())
        .bind(user.as_i64())
        .execute(conn)
        .await
        .map_err(db)?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(Resource::Application));
    }
    Ok(())
}
