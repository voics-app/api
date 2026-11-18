//! Application, bot identity and command reads.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::FromRow;

use application::ports::{ApplicationRepository, BotIdentity};
use application::{AppError, AppResult};
use domain::application::{Application, ApplicationCommand, CommandKind};
use domain::Snowflake;

use super::convert::{permissions, sf, sf_opt};
use super::error::db;
use super::PgStore;

const APPLICATION_COLUMNS: &str =
    "id, name, description, icon, owner_id, bot_user_id, bot_public, \
     bot_require_code_grant, verify_key, flags, interactions_endpoint_url, created_at";

const COMMAND_COLUMNS: &str = "id, application_id, guild_id, name, description, kind, options, \
     default_member_permissions, dm_permission, nsfw, version";

#[derive(FromRow)]
pub(super) struct ApplicationRow {
    id: i64,
    name: String,
    description: String,
    icon: Option<String>,
    owner_id: i64,
    bot_user_id: i64,
    bot_public: bool,
    bot_require_code_grant: bool,
    verify_key: String,
    flags: i64,
    interactions_endpoint_url: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<ApplicationRow> for Application {
    fn from(row: ApplicationRow) -> Self {
        Application {
            id: sf(row.id),
            name: row.name,
            description: row.description,
            icon: row.icon,
            owner_id: sf(row.owner_id),
            bot_user_id: sf(row.bot_user_id),
            bot_public: row.bot_public,
            bot_require_code_grant: row.bot_require_code_grant,
            verify_key: row.verify_key,
            flags: row.flags as u64,
            interactions_endpoint_url: row.interactions_endpoint_url,
            created_at: row.created_at,
        }
    }
}

#[derive(FromRow)]
struct CommandRow {
    id: i64,
    application_id: i64,
    guild_id: Option<i64>,
    name: String,
    description: String,
    kind: i16,
    options: serde_json::Value,
    default_member_permissions: Option<i64>,
    dm_permission: bool,
    nsfw: bool,
    version: i64,
}

impl TryFrom<CommandRow> for ApplicationCommand {
    type Error = AppError;

    fn try_from(row: CommandRow) -> Result<Self, Self::Error> {
        let kind = CommandKind::from_code(row.kind as u8)
            .ok_or_else(|| AppError::internal(format!("unknown command kind {}", row.kind)))?;
        Ok(ApplicationCommand {
            id: sf(row.id),
            application_id: sf(row.application_id),
            guild_id: sf_opt(row.guild_id),
            name: row.name,
            description: row.description,
            kind,
            options: row.options,
            default_member_permissions: row.default_member_permissions.map(permissions),
            dm_permission: row.dm_permission,
            nsfw: row.nsfw,
            version: sf(row.version),
        })
    }
}

#[async_trait]
impl ApplicationRepository for PgStore {
    async fn installation_revision(
        &self,
        guild_id: Snowflake,
        application_id: Snowflake,
    ) -> AppResult<Option<i64>> {
        sqlx::query_scalar("SELECT last_generation FROM application_installation_versions WHERE guild_id=$1 AND application_id=$2")
            .bind(guild_id.as_i64()).bind(application_id.as_i64()).fetch_optional(&self.pool).await.map_err(db)
    }
    async fn installation(
        &self,
        guild_id: Snowflake,
        application_id: Snowflake,
    ) -> AppResult<Option<domain::bot_authorization::BotInstallation>> {
        super::authorizations::installation(&self.pool, guild_id, application_id).await
    }
    async fn authorizations(
        &self,
        user_id: Snowflake,
    ) -> AppResult<Vec<domain::bot_authorization::BotAuthorization>> {
        super::authorizations::authorizations(&self.pool, user_id, None).await
    }
    async fn authorization(
        &self,
        user_id: Snowflake,
        id: Snowflake,
    ) -> AppResult<Option<domain::bot_authorization::BotAuthorization>> {
        Ok(
            super::authorizations::authorizations(&self.pool, user_id, Some(id))
                .await?
                .into_iter()
                .next(),
        )
    }
    async fn find(&self, id: Snowflake) -> AppResult<Option<Application>> {
        let sql = format!("SELECT {APPLICATION_COLUMNS} FROM applications WHERE id = $1");
        let row: Option<ApplicationRow> = sqlx::query_as(&sql)
            .bind(id.as_i64())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        Ok(row.map(Application::from))
    }

    async fn find_by_bot(&self, bot_user_id: Snowflake) -> AppResult<Option<Application>> {
        let sql = format!("SELECT {APPLICATION_COLUMNS} FROM applications WHERE bot_user_id = $1");
        let row: Option<ApplicationRow> = sqlx::query_as(&sql)
            .bind(bot_user_id.as_i64())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        Ok(row.map(Application::from))
    }

    async fn list_for_owner(&self, owner_id: Snowflake) -> AppResult<Vec<Application>> {
        let sql = format!(
            "SELECT {APPLICATION_COLUMNS} FROM applications WHERE owner_id = $1 ORDER BY id"
        );
        let rows: Vec<ApplicationRow> = sqlx::query_as(&sql)
            .bind(owner_id.as_i64())
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        Ok(rows.into_iter().map(Application::from).collect())
    }

    async fn bot_identity(&self, token_hash: &str) -> AppResult<Option<BotIdentity>> {
        let row: Option<(i64, i64)> = sqlx::query_as(
            "SELECT bot_user_id, application_id FROM bot_tokens WHERE token_hash = $1",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row.map(|(user_id, application_id)| BotIdentity {
            user_id: sf(user_id),
            application_id: sf(application_id),
        }))
    }

    /// `guild_id: None` lists global commands only; `Some` lists that guild's commands only.
    async fn commands(
        &self,
        application_id: Snowflake,
        guild_id: Option<Snowflake>,
    ) -> AppResult<Vec<ApplicationCommand>> {
        let sql = format!(
            "SELECT {COMMAND_COLUMNS} FROM application_commands \
             WHERE application_id = $1 AND guild_id IS NOT DISTINCT FROM $2 ORDER BY id"
        );
        let rows: Vec<CommandRow> = sqlx::query_as(&sql)
            .bind(application_id.as_i64())
            .bind(guild_id.map(|id| id.as_i64()))
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        rows.into_iter().map(ApplicationCommand::try_from).collect()
    }

    async fn command(
        &self,
        application_id: Snowflake,
        command_id: Snowflake,
    ) -> AppResult<Option<ApplicationCommand>> {
        let sql = format!("SELECT {COMMAND_COLUMNS} FROM application_commands WHERE application_id = $1 AND id = $2");
        let row: Option<CommandRow> = sqlx::query_as(&sql)
            .bind(application_id.as_i64())
            .bind(command_id.as_i64())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        row.map(ApplicationCommand::try_from).transpose()
    }

    async fn installed_in_guild(&self, guild_id: Snowflake) -> AppResult<Vec<Application>> {
        let sql = format!(
            "SELECT a.{cols} FROM applications a JOIN members m ON m.user_id = a.bot_user_id \
             WHERE m.guild_id = $1 ORDER BY a.id",
            cols = APPLICATION_COLUMNS.replace(", ", ", a.")
        );
        let rows: Vec<ApplicationRow> = sqlx::query_as(&sql)
            .bind(guild_id.as_i64())
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        Ok(rows.into_iter().map(Application::from).collect())
    }
}
