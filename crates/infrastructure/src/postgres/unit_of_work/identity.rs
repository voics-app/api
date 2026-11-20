//! Writes for accounts, credentials, settings, relationships and channels.

use sqlx::PgConnection;

use application::AppResult;
use domain::channel::Channel;
use domain::delivery_mode::DeliveryMode;
use domain::notification::GuildNotificationSettings;
use domain::permissions::OverwriteKind;
use domain::relationship::RelationshipType;
use domain::user::{Credentials, Suspension, User, UserSettings};
use domain::Snowflake;

use crate::postgres::convert::{bits, custom_status_json, thread_metadata_json};
use crate::postgres::error::db;
use crate::postgres::notifications;

pub(super) async fn upsert_user(conn: &mut PgConnection, user: &User) -> AppResult<()> {
    let (suspended, reason, until) = suspension_columns(user.suspension.as_ref());
    sqlx::query(
        "INSERT INTO users (id, username, email, global_name, avatar, bio, bot, system, verified, mfa_enabled, flags, \
             platform_admin, suspended, suspension_reason, suspended_until, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16) \
         ON CONFLICT (id) DO UPDATE SET username = EXCLUDED.username, email = EXCLUDED.email, \
             global_name = EXCLUDED.global_name, avatar = EXCLUDED.avatar, bio = EXCLUDED.bio, verified = EXCLUDED.verified, \
             flags = EXCLUDED.flags, platform_admin = EXCLUDED.platform_admin, \
             suspended = EXCLUDED.suspended, suspension_reason = EXCLUDED.suspension_reason, \
             suspended_until = EXCLUDED.suspended_until",
    )
    .bind(user.id.as_i64())
    .bind(&user.username)
    .bind(&user.email)
    .bind(&user.global_name)
    .bind(&user.avatar)
    .bind(&user.bio)
    .bind(user.bot)
    .bind(user.system)
    .bind(user.verified)
    .bind(user.mfa_enabled)
    .bind(user.flags as i64)
    .bind(user.platform_admin)
    .bind(suspended)
    .bind(reason)
    .bind(until)
    .bind(user.created_at)
    .execute(conn)
    .await
    .map_err(db)?;
    Ok(())
}

fn suspension_columns(
    suspension: Option<&Suspension>,
) -> (bool, Option<String>, Option<chrono::DateTime<chrono::Utc>>) {
    match suspension {
        Some(s) => (true, Some(s.reason.clone()), s.until),
        None => (false, None, None),
    }
}

pub(super) async fn insert_credentials(
    conn: &mut PgConnection,
    credentials: &Credentials,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO user_credentials (user_id, password_hash, totp_secret, pending_totp_secret) VALUES ($1, $2, $3, $4)",
    )
    .bind(credentials.user_id.as_i64())
    .bind(&credentials.password_hash)
    .bind(&credentials.totp_secret)
    .bind(&credentials.pending_totp_secret)
    .execute(conn)
    .await
    .map_err(db)?;
    Ok(())
}

pub(super) async fn set_totp_secret(
    conn: &mut PgConnection,
    user_id: Snowflake,
    secret: Option<&str>,
) -> AppResult<()> {
    // Factor changes share the account lock with passkey activation/removal.
    super::super::passkeys::lock_account(conn, user_id).await?;
    sqlx::query(
        "UPDATE user_credentials SET totp_secret = $2, pending_totp_secret = NULL WHERE user_id = $1",
    )
    .bind(user_id.as_i64())
    .bind(secret)
    .execute(&mut *conn)
    .await
    .map_err(db)?;
    super::super::passkeys::sync_mfa(conn, user_id).await
}

pub(super) async fn upsert_settings(
    conn: &mut PgConnection,
    settings: &UserSettings,
    expected_revision: Option<i64>,
) -> AppResult<()> {
    let result = sqlx::query(
        "INSERT INTO user_settings (user_id, default_delivery_mode, e2ee_calls_default, theme, locale, \
             status, custom_status, developer_mode, accent_color, guild_folders, guild_positions, \
             message_display_compact, preferences, pinned_dm_channel_ids, pinned_channel_ids, friend_source_all, friend_source_mutual_friends, friend_source_mutual_guilds, settings_revision) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, CASE WHEN $19::bigint IS NOT NULL THEN 1 ELSE 0 END) \
         ON CONFLICT (user_id) DO UPDATE SET default_delivery_mode = EXCLUDED.default_delivery_mode, \
             e2ee_calls_default = EXCLUDED.e2ee_calls_default, theme = EXCLUDED.theme, locale = EXCLUDED.locale, status = EXCLUDED.status, \
             custom_status = EXCLUDED.custom_status, developer_mode = EXCLUDED.developer_mode, \
             accent_color = EXCLUDED.accent_color, guild_folders = EXCLUDED.guild_folders, \
             guild_positions = EXCLUDED.guild_positions, \
             message_display_compact = EXCLUDED.message_display_compact, preferences = EXCLUDED.preferences, \n             pinned_dm_channel_ids = EXCLUDED.pinned_dm_channel_ids, pinned_channel_ids = EXCLUDED.pinned_channel_ids, friend_source_all = EXCLUDED.friend_source_all, friend_source_mutual_friends = EXCLUDED.friend_source_mutual_friends, friend_source_mutual_guilds = EXCLUDED.friend_source_mutual_guilds \
             WHERE $19::bigint IS NULL OR user_settings.settings_revision = $19",
    )
    .bind(settings.user_id.as_i64())
    .bind(settings.default_delivery_mode.as_str())
    .bind(settings.e2ee_calls_default)
    .bind(settings.theme.as_str())
    .bind(&settings.locale)
    .bind(settings.status.as_str())
    .bind(settings.custom_status.as_ref().map(custom_status_json))
    .bind(settings.developer_mode)
    .bind(settings.accent_color.map(|c| c as i32))
    .bind(super::super::convert::guild_folders_json(&settings.guild_folders))
    .bind(
        settings
            .guild_positions
            .iter()
            .map(|id| id.as_i64())
            .collect::<Vec<_>>(),
    )
    .bind(settings.message_display_compact)
    .bind(serde_json::Value::Object(settings.preferences.clone()))
    .bind(
        settings
            .pinned_dm_channel_ids
            .iter()
            .map(|id| id.as_i64())
            .collect::<Vec<_>>(),
    )
    .bind(super::super::convert::pinned_channels_json(
        &settings.pinned_channel_ids,
    ))
    .bind(settings.friend_source_flags.all)
    .bind(settings.friend_source_flags.mutual_friends)
    .bind(settings.friend_source_flags.mutual_guilds)
    .bind(expected_revision)
    .execute(conn)
    .await
    .map_err(db)?;
    if result.rows_affected() == 0 {
        return Err(application::AppError::SettingsWriteConflict);
    }
    Ok(())
}

pub(super) async fn upsert_guild_notification_settings(
    conn: &mut PgConnection,
    settings: &GuildNotificationSettings,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO user_guild_settings (user_id, guild_id, message_notifications, suppress_everyone, \
             suppress_roles, muted, mute_end_time, hide_muted_channels, channel_overrides, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now()) \
         ON CONFLICT (user_id, guild_id) DO UPDATE SET message_notifications = EXCLUDED.message_notifications, \
             suppress_everyone = EXCLUDED.suppress_everyone, suppress_roles = EXCLUDED.suppress_roles, \
             muted = EXCLUDED.muted, mute_end_time = EXCLUDED.mute_end_time, \
             hide_muted_channels = EXCLUDED.hide_muted_channels, \
             channel_overrides = EXCLUDED.channel_overrides, updated_at = now()",
    )
    .bind(settings.user_id.as_i64())
    .bind(settings.guild_id.as_i64())
    .bind(settings.level.map(|l| i16::from(l.code())))
    .bind(settings.suppress_everyone)
    .bind(settings.suppress_roles)
    .bind(settings.mute.muted)
    .bind(settings.mute.end_time)
    .bind(settings.hide_muted_channels)
    .bind(notifications::overrides_json(&settings.channel_overrides))
    .execute(conn)
    .await
    .map_err(db)?;
    Ok(())
}

pub(super) async fn set_user_note(
    conn: &mut PgConnection,
    owner_id: Snowflake,
    target_id: Snowflake,
    note: Option<&str>,
) -> AppResult<()> {
    match note {
        Some(note) => {
            sqlx::query(
                "INSERT INTO user_notes (owner_id, target_id, note) VALUES ($1, $2, $3) \
                 ON CONFLICT (owner_id, target_id) DO UPDATE SET note = EXCLUDED.note",
            )
            .bind(owner_id.as_i64())
            .bind(target_id.as_i64())
            .bind(note)
            .execute(conn)
            .await
            .map_err(db)?;
        }
        None => {
            sqlx::query("DELETE FROM user_notes WHERE owner_id = $1 AND target_id = $2")
                .bind(owner_id.as_i64())
                .bind(target_id.as_i64())
                .execute(conn)
                .await
                .map_err(db)?;
        }
    }
    Ok(())
}

pub(super) async fn set_relationship(
    conn: &mut PgConnection,
    user_id: Snowflake,
    target_id: Snowflake,
    kind: Option<RelationshipType>,
    since: chrono::DateTime<chrono::Utc>,
) -> AppResult<()> {
    match kind {
        Some(kind) => {
            sqlx::query(
                "INSERT INTO relationships (user_id, target_id, kind, since) VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (user_id, target_id) DO UPDATE SET kind = EXCLUDED.kind, since = EXCLUDED.since",
            )
            .bind(user_id.as_i64())
            .bind(target_id.as_i64())
            .bind(i16::from(kind.code()))
            .bind(since)
            .execute(conn)
            .await
            .map_err(db)?;
        }
        None => {
            sqlx::query("DELETE FROM relationships WHERE user_id = $1 AND target_id = $2")
                .bind(user_id.as_i64())
                .bind(target_id.as_i64())
                .execute(conn)
                .await
                .map_err(db)?;
        }
    }
    Ok(())
}

pub(super) async fn suspend_user(
    conn: &mut PgConnection,
    user_id: Snowflake,
    suspension: &Suspension,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE users SET suspended = TRUE, suspension_reason = $2, suspended_until = $3 WHERE id = $1",
    )
    .bind(user_id.as_i64())
    .bind(&suspension.reason)
    .bind(suspension.until)
    .execute(conn)
    .await
    .map_err(db)?;
    Ok(())
}

pub(super) async fn upsert_channel(conn: &mut PgConnection, channel: &Channel) -> AppResult<()> {
    crate::postgres::community::check_update(conn, channel).await?;
    sqlx::query(
        "INSERT INTO channels (id, kind, guild_id, position, name, topic, nsfw, text_chat_enabled, bitrate, user_limit, \
             rate_limit_per_user, parent_id, owner_id, icon, delivery_mode, voice_region, thread_metadata, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18) \
         ON CONFLICT (id) DO UPDATE SET kind = EXCLUDED.kind, guild_id = EXCLUDED.guild_id, \
             position = EXCLUDED.position, name = EXCLUDED.name, topic = EXCLUDED.topic, nsfw = EXCLUDED.nsfw, \
             text_chat_enabled = EXCLUDED.text_chat_enabled, bitrate = EXCLUDED.bitrate, user_limit = EXCLUDED.user_limit, \
             rate_limit_per_user = EXCLUDED.rate_limit_per_user, parent_id = EXCLUDED.parent_id, \
             owner_id = EXCLUDED.owner_id, icon = EXCLUDED.icon, delivery_mode = EXCLUDED.delivery_mode, \
             voice_region = EXCLUDED.voice_region, thread_metadata = EXCLUDED.thread_metadata",
    )
    .bind(channel.id.as_i64())
    .bind(i16::from(channel.kind.code()))
    .bind(channel.guild_id.map(|id| id.as_i64()))
    .bind(channel.position)
    .bind(&channel.name)
    .bind(&channel.topic)
    .bind(channel.nsfw)
    .bind(channel.text_chat_enabled)
    .bind(channel.bitrate.map(|v| v as i32))
    .bind(channel.user_limit.map(|v| v as i32))
    .bind(channel.rate_limit_per_user as i32)
    .bind(channel.parent_id.map(|id| id.as_i64()))
    .bind(channel.owner_id.map(|id| id.as_i64()))
    .bind(&channel.icon)
    .bind(channel.delivery_mode.as_str())
    .bind(&channel.voice_region)
    .bind(channel.thread_metadata.as_ref().map(thread_metadata_json))
    .bind(channel.created_at)
    .execute(&mut *conn)
    .await
    .map_err(db)?;

    sqlx::query("DELETE FROM channel_recipients WHERE channel_id = $1")
        .bind(channel.id.as_i64())
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    for recipient in &channel.recipient_ids {
        sqlx::query("INSERT INTO channel_recipients (channel_id, user_id) VALUES ($1, $2)")
            .bind(channel.id.as_i64())
            .bind(recipient.as_i64())
            .execute(&mut *conn)
            .await
            .map_err(db)?;
    }

    sqlx::query("DELETE FROM permission_overwrites WHERE channel_id = $1")
        .bind(channel.id.as_i64())
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    for overwrite in &channel.overwrites {
        let kind = match overwrite.kind {
            OverwriteKind::Role => 0_i16,
            OverwriteKind::Member => 1_i16,
        };
        sqlx::query(
            "INSERT INTO permission_overwrites (channel_id, target_id, kind, allow_bits, deny_bits) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(channel.id.as_i64())
        .bind(overwrite.id.as_i64())
        .bind(kind)
        .bind(bits(overwrite.allow))
        .bind(bits(overwrite.deny))
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    }
    Ok(())
}

pub(super) async fn delete_channel(
    conn: &mut PgConnection,
    channel_id: Snowflake,
) -> AppResult<()> {
    crate::postgres::community::check_delete(conn, channel_id).await?;
    sqlx::query("DELETE FROM channels WHERE id = $1")
        .bind(channel_id.as_i64())
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}

pub(super) async fn set_delivery_override(
    conn: &mut PgConnection,
    user_id: Snowflake,
    channel_id: Snowflake,
    mode: Option<DeliveryMode>,
) -> AppResult<()> {
    match mode {
        Some(mode) => {
            sqlx::query(
                "INSERT INTO channel_delivery_overrides (user_id, channel_id, mode) VALUES ($1, $2, $3) \
                 ON CONFLICT (user_id, channel_id) DO UPDATE SET mode = EXCLUDED.mode",
            )
            .bind(user_id.as_i64())
            .bind(channel_id.as_i64())
            .bind(mode.as_str())
            .execute(conn)
            .await
            .map_err(db)?;
        }
        None => {
            sqlx::query(
                "DELETE FROM channel_delivery_overrides WHERE user_id = $1 AND channel_id = $2",
            )
            .bind(user_id.as_i64())
            .bind(channel_id.as_i64())
            .execute(conn)
            .await
            .map_err(db)?;
        }
    }
    Ok(())
}

pub(super) async fn set_password_hash(
    conn: &mut PgConnection,
    user_id: Snowflake,
    hash: &str,
) -> AppResult<()> {
    sqlx::query("UPDATE user_credentials SET password_hash = $2 WHERE user_id = $1")
        .bind(user_id.as_i64())
        .bind(hash)
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}

pub(super) async fn set_pending_totp(
    conn: &mut PgConnection,
    user_id: Snowflake,
    secret: Option<&str>,
) -> AppResult<()> {
    sqlx::query("UPDATE user_credentials SET pending_totp_secret = $2 WHERE user_id = $1")
        .bind(user_id.as_i64())
        .bind(secret)
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}

pub(super) async fn replace_backup_codes(
    conn: &mut PgConnection,
    user_id: Snowflake,
    hashes: &[String],
) -> AppResult<()> {
    sqlx::query("DELETE FROM user_backup_codes WHERE user_id = $1")
        .bind(user_id.as_i64())
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    for hash in hashes {
        sqlx::query("INSERT INTO user_backup_codes (user_id, code_hash) VALUES ($1, $2)")
            .bind(user_id.as_i64())
            .bind(hash)
            .execute(&mut *conn)
            .await
            .map_err(db)?;
    }
    Ok(())
}

pub(super) async fn set_account_state(
    conn: &mut PgConnection,
    user_id: Snowflake,
    state: &domain::account::AccountState,
) -> AppResult<()> {
    sqlx::query("UPDATE users SET disabled_at = $2, delete_after = $3 WHERE id = $1")
        .bind(user_id.as_i64())
        .bind(state.disabled_at)
        .bind(state.delete_after)
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}

pub(super) async fn anonymize_user(
    conn: &mut PgConnection,
    user_id: Snowflake,
    username: &str,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE users SET username = $2, email = NULL, global_name = NULL, avatar = NULL, bio = NULL, \
             banner = NULL, pronouns = NULL, theme_colors = NULL, accent_color = NULL, \
             verified = FALSE, mfa_enabled = FALSE, delete_after = NULL, \
             disabled_at = COALESCE(disabled_at, now()) WHERE id = $1",
    )
    .bind(user_id.as_i64())
    .bind(username)
    .execute(&mut *conn)
    .await
    .map_err(db)?;
    sqlx::query("UPDATE members SET nick = NULL, avatar = NULL, banner = NULL, bio = NULL WHERE user_id = $1")
        .bind(user_id.as_i64())
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    for table in [
        "user_credentials",
        "user_backup_codes",
        "account_tokens",
        "user_passkeys",
        "passkey_challenges",
        "application_authorizations",
        "user_badges",
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE user_id = $1"))
            .bind(user_id.as_i64())
            .execute(&mut *conn)
            .await
            .map_err(db)?;
    }
    Ok(())
}
