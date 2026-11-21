use super::{
    convert::{permissions, sf},
    error::db,
};
use application::AppResult;
use chrono::{DateTime, Utc};
use domain::{
    bot_authorization::{BotAuthorization, BotInstallation},
    Snowflake,
};
use sqlx::{FromRow, PgPool};

#[derive(FromRow)]
struct InstallationRow {
    generation: i64,
    guild_id: i64,
    application_id: i64,
    bot_user_id: i64,
    installed_at: DateTime<Utc>,
}
impl From<InstallationRow> for BotInstallation {
    fn from(row: InstallationRow) -> Self {
        Self {
            generation: row.generation,
            guild_id: sf(row.guild_id),
            application_id: sf(row.application_id),
            bot_user_id: sf(row.bot_user_id),
            installed_at: row.installed_at,
        }
    }
}

pub(super) async fn installation(
    pool: &PgPool,
    guild: Snowflake,
    app: Snowflake,
) -> AppResult<Option<BotInstallation>> {
    let row: Option<InstallationRow> = sqlx::query_as("SELECT generation, guild_id, application_id, bot_user_id, installed_at FROM guild_application_installations WHERE guild_id=$1 AND application_id=$2")
        .bind(guild.as_i64()).bind(app.as_i64()).fetch_optional(pool).await.map_err(db)?;
    Ok(row.map(Into::into))
}

#[derive(FromRow)]
struct AuthorizationRow {
    id: i64,
    user_id: i64,
    scopes: Vec<String>,
    permissions: i64,
    created_at: DateTime<Utc>,
    authorized_at: DateTime<Utc>,
    #[sqlx(flatten)]
    installation: InstallationRow,
}
impl From<AuthorizationRow> for BotAuthorization {
    fn from(row: AuthorizationRow) -> Self {
        Self {
            id: sf(row.id),
            user_id: sf(row.user_id),
            installation: row.installation.into(),
            scopes: row.scopes,
            permissions: permissions(row.permissions),
            created_at: row.created_at,
            authorized_at: row.authorized_at,
        }
    }
}

pub(super) async fn authorizations(
    pool: &PgPool,
    user: Snowflake,
    id: Option<Snowflake>,
) -> AppResult<Vec<BotAuthorization>> {
    let rows: Vec<AuthorizationRow> = sqlx::query_as("SELECT a.id, a.user_id, a.scopes, a.permissions, a.created_at, a.authorized_at, i.generation, i.guild_id, i.application_id, i.bot_user_id, i.installed_at FROM application_authorizations a JOIN guild_application_installations i ON i.generation=a.installation_generation WHERE a.user_id=$1 AND ($2::bigint IS NULL OR a.id=$2) ORDER BY a.authorized_at DESC, a.id DESC")
        .bind(user.as_i64()).bind(id.map(|id| id.as_i64())).fetch_all(pool).await.map_err(db)?;
    Ok(rows.into_iter().map(Into::into).collect())
}
