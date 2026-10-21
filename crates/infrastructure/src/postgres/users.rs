use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::FromRow;

use application::ports::{UserRepository, UserStats};
use application::AppResult;
use domain::account::AccountState;
use domain::mfa::BackupCodeRecord;
use domain::notification::GuildNotificationSettings;
use domain::user::{Credentials, PresenceStatus, Suspension, Theme, User, UserSettings};
use domain::Snowflake;

use super::convert::{custom_status_from_json, like_pattern, mode, raw_ids, sf, sfs};
use super::error::db;
use super::PgStore;

pub(super) const USER_COLUMNS: &str =
    "id, username, email, global_name, avatar, bot, system, verified, \
     mfa_enabled, flags, platform_admin, suspended, suspension_reason, suspended_until, created_at";

#[derive(FromRow)]
pub(super) struct UserRow {
    id: i64,
    username: String,
    email: Option<String>,
    global_name: Option<String>,
    avatar: Option<String>,
    bot: bool,
    system: bool,
    verified: bool,
    mfa_enabled: bool,
    flags: i64,
    platform_admin: bool,
    suspended: bool,
    suspension_reason: Option<String>,
    suspended_until: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

impl From<UserRow> for User {
    fn from(row: UserRow) -> Self {
        User {
            id: sf(row.id),
            username: row.username,
            global_name: row.global_name,
            avatar: row.avatar,
            bot: row.bot,
            system: row.system,
            email: row.email,
            verified: row.verified,
            mfa_enabled: row.mfa_enabled,
            flags: row.flags as u64,
            platform_admin: row.platform_admin,
            suspension: row.suspended.then(|| Suspension {
                reason: row.suspension_reason.unwrap_or_default(),
                until: row.suspended_until,
            }),
            created_at: row.created_at,
        }
    }
}

#[derive(FromRow)]
struct SettingsRow {
    user_id: i64,
    default_delivery_mode: String,
    e2ee_calls_default: bool,
    theme: String,
    locale: String,
    status: String,
    custom_status: Option<serde_json::Value>,
    developer_mode: bool,
    accent_color: Option<i32>,
}

impl From<SettingsRow> for UserSettings {
    fn from(row: SettingsRow) -> Self {
        UserSettings {
            user_id: sf(row.user_id),
            default_delivery_mode: mode(&row.default_delivery_mode),
            e2ee_calls_default: row.e2ee_calls_default,
            theme: Theme::parse(&row.theme).unwrap_or_default(),
            locale: row.locale,
            status: PresenceStatus::parse(&row.status).unwrap_or_default(),
            custom_status: row.custom_status.and_then(custom_status_from_json),
            developer_mode: row.developer_mode,
            accent_color: row.accent_color.map(|c| c as u32),
        }
    }
}

const SETTINGS_COLUMNS: &str =
    "user_id, default_delivery_mode, e2ee_calls_default, theme, locale, status, custom_status, developer_mode, accent_color";

#[async_trait]
impl UserRepository for PgStore {
    async fn find(&self, id: Snowflake) -> AppResult<Option<User>> {
        let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE id = $1");
        let row: Option<UserRow> = sqlx::query_as(&sql)
            .bind(id.as_i64())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        Ok(row.map(User::from))
    }

    async fn find_many(&self, ids: &[Snowflake]) -> AppResult<Vec<User>> {
        let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE id = ANY($1) ORDER BY id");
        let rows: Vec<UserRow> = sqlx::query_as(&sql)
            .bind(raw_ids(ids))
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        Ok(rows.into_iter().map(User::from).collect())
    }

    async fn find_by_email(&self, email: &str) -> AppResult<Option<User>> {
        let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE lower(email) = lower($1)");
        let row: Option<UserRow> = sqlx::query_as(&sql)
            .bind(email)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        Ok(row.map(User::from))
    }

    async fn find_by_username(&self, username: &str) -> AppResult<Option<User>> {
        let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE lower(username) = lower($1)");
        let row: Option<UserRow> = sqlx::query_as(&sql)
            .bind(username)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        Ok(row.map(User::from))
    }

    async fn credentials(&self, user_id: Snowflake) -> AppResult<Option<Credentials>> {
        let row: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT password_hash, totp_secret, pending_totp_secret FROM user_credentials WHERE user_id = $1",
        )
        .bind(user_id.as_i64())
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row.map(
            |(password_hash, totp_secret, pending_totp_secret)| Credentials {
                user_id,
                password_hash,
                totp_secret,
                pending_totp_secret,
            },
        ))
    }

    async fn account_state(&self, user_id: Snowflake) -> AppResult<AccountState> {
        let row: Option<(Option<DateTime<Utc>>, Option<DateTime<Utc>>)> =
            sqlx::query_as("SELECT disabled_at, delete_after FROM users WHERE id = $1")
                .bind(user_id.as_i64())
                .fetch_optional(&self.pool)
                .await
                .map_err(db)?;
        Ok(row
            .map(|(disabled_at, delete_after)| AccountState {
                disabled_at,
                delete_after,
            })
            .unwrap_or_default())
    }

    async fn backup_codes(&self, user_id: Snowflake) -> AppResult<Vec<BackupCodeRecord>> {
        let rows: Vec<(i64, String, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT id, code_hash, consumed_at FROM user_backup_codes WHERE user_id = $1 ORDER BY id",
        )
        .bind(user_id.as_i64())
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows
            .into_iter()
            .map(|(id, hash, consumed_at)| BackupCodeRecord {
                id,
                hash,
                consumed: consumed_at.is_some(),
            })
            .collect())
    }

    async fn consume_backup_code(&self, id: i64, now: DateTime<Utc>) -> AppResult<bool> {
        let result = sqlx::query(
            "UPDATE user_backup_codes SET consumed_at = $2 WHERE id = $1 AND consumed_at IS NULL",
        )
        .bind(id)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(result.rows_affected() == 1)
    }

    async fn list_due_deletions(
        &self,
        now: DateTime<Utc>,
        limit: u32,
    ) -> AppResult<Vec<Snowflake>> {
        let rows: Vec<(i64,)> = sqlx::query_as(
            "SELECT id FROM users WHERE delete_after IS NOT NULL AND delete_after <= $1 ORDER BY delete_after LIMIT $2",
        )
        .bind(now)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows.into_iter().map(|(id,)| sf(id)).collect())
    }

    async fn settings(&self, user_id: Snowflake) -> AppResult<UserSettings> {
        let sql = format!("SELECT {SETTINGS_COLUMNS} FROM user_settings WHERE user_id = $1");
        let row: Option<SettingsRow> = sqlx::query_as(&sql)
            .bind(user_id.as_i64())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        Ok(row
            .map(UserSettings::from)
            .unwrap_or_else(|| UserSettings::defaults_for(user_id)))
    }

    async fn settings_many(&self, user_ids: &[Snowflake]) -> AppResult<Vec<UserSettings>> {
        let sql = format!("SELECT {SETTINGS_COLUMNS} FROM user_settings WHERE user_id = ANY($1)");
        let rows: Vec<SettingsRow> = sqlx::query_as(&sql)
            .bind(raw_ids(user_ids))
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        Ok(rows.into_iter().map(UserSettings::from).collect())
    }

    async fn guild_notification_settings(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
    ) -> AppResult<Option<GuildNotificationSettings>> {
        let sql = format!(
            "SELECT {} FROM user_guild_settings WHERE user_id = $1 AND guild_id = $2",
            super::notifications::COLUMNS
        );
        let row: Option<super::notifications::Row> = sqlx::query_as(&sql)
            .bind(user_id.as_i64())
            .bind(guild_id.as_i64())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        Ok(row.map(GuildNotificationSettings::from))
    }

    async fn list_guild_notification_settings(
        &self,
        user_id: Snowflake,
    ) -> AppResult<Vec<GuildNotificationSettings>> {
        let sql = format!(
            "SELECT {} FROM user_guild_settings WHERE user_id = $1 ORDER BY guild_id",
            super::notifications::COLUMNS
        );
        let rows: Vec<super::notifications::Row> = sqlx::query_as(&sql)
            .bind(user_id.as_i64())
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        Ok(rows
            .into_iter()
            .map(GuildNotificationSettings::from)
            .collect())
    }

    async fn note(&self, owner_id: Snowflake, target_id: Snowflake) -> AppResult<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT note FROM user_notes WHERE owner_id = $1 AND target_id = $2")
                .bind(owner_id.as_i64())
                .bind(target_id.as_i64())
                .fetch_optional(&self.pool)
                .await
                .map_err(db)?;
        Ok(row.map(|(note,)| note))
    }

    async fn notes(&self, owner_id: Snowflake) -> AppResult<Vec<(Snowflake, String)>> {
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT target_id, note FROM user_notes WHERE owner_id = $1 ORDER BY target_id",
        )
        .bind(owner_id.as_i64())
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows.into_iter().map(|(id, note)| (sf(id), note)).collect())
    }

    async fn search(
        &self,
        query: Option<&str>,
        limit: u32,
        after: Option<Snowflake>,
    ) -> AppResult<Vec<User>> {
        let sql = format!(
            "SELECT {USER_COLUMNS} FROM users \
             WHERE ($1::text IS NULL OR username ILIKE $1 OR email ILIKE $1) \
               AND ($2::bigint IS NULL OR id > $2) \
             ORDER BY id LIMIT $3"
        );
        let rows: Vec<UserRow> = sqlx::query_as(&sql)
            .bind(query.map(like_pattern))
            .bind(after.map(|id| id.as_i64()))
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        Ok(rows.into_iter().map(User::from).collect())
    }

    async fn co_member_ids(&self, user_id: Snowflake) -> AppResult<Vec<Snowflake>> {
        let rows: Vec<(i64,)> = sqlx::query_as(
            "SELECT r.user_id FROM channel_recipients mine \
                 JOIN channel_recipients r ON r.channel_id = mine.channel_id WHERE mine.user_id = $1 \
             UNION \
             SELECT m.user_id FROM members mine \
                 JOIN members m ON m.guild_id = mine.guild_id WHERE mine.user_id = $1",
        )
        .bind(user_id.as_i64())
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(sfs(rows.into_iter().map(|(id,)| id).collect()))
    }

    async fn stats(&self, signups_since: DateTime<Utc>) -> AppResult<UserStats> {
        let (users, bots, signups): (i64, i64, i64) = sqlx::query_as(
            "SELECT count(*) FILTER (WHERE NOT bot), count(*) FILTER (WHERE bot), \
                    count(*) FILTER (WHERE NOT bot AND created_at >= $1) FROM users",
        )
        .bind(signups_since)
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;
        Ok(UserStats {
            users_total: users as u64,
            bots_total: bots as u64,
            signups_since: signups as u64,
        })
    }
}
