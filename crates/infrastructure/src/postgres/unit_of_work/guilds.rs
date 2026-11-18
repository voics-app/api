//! Writes for guilds, roles, members, bans, invites, emojis and the audit log.

use sqlx::PgConnection;

use application::{AppError, AppResult};
use contracts::errors::UNKNOWN_INVITE;
use domain::audit::AuditLogEntry;
use domain::emoji::Emoji;
use domain::guild::{Guild, Member, Role};
use domain::invite::Invite;
use domain::Snowflake;

use crate::postgres::convert::{bits, raw_ids};
use crate::postgres::error::db;

pub(super) async fn upsert_guild(conn: &mut PgConnection, guild: &Guild) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO guilds (id, name, icon, description, owner_id, voice_region, verification_level, \
             default_message_notifications, explicit_content_filter, afk_channel_id, afk_timeout, \
             system_channel_id, system_channel_flags, banner, splash, prune_admin_only, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17) \
         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, icon = EXCLUDED.icon, \
             description = EXCLUDED.description, owner_id = EXCLUDED.owner_id, \
             voice_region = EXCLUDED.voice_region, verification_level = EXCLUDED.verification_level, \
             default_message_notifications = EXCLUDED.default_message_notifications, \
             explicit_content_filter = EXCLUDED.explicit_content_filter, afk_channel_id = EXCLUDED.afk_channel_id, \
             afk_timeout = EXCLUDED.afk_timeout, system_channel_id = EXCLUDED.system_channel_id, \n             system_channel_flags = EXCLUDED.system_channel_flags, banner = EXCLUDED.banner, \n             splash = EXCLUDED.splash, prune_admin_only = EXCLUDED.prune_admin_only",
    )
    .bind(guild.id.as_i64())
    .bind(&guild.name)
    .bind(&guild.icon)
    .bind(&guild.description)
    .bind(guild.owner_id.as_i64())
    .bind(&guild.voice_region)
    .bind(i16::from(guild.verification_level))
    .bind(i16::from(guild.default_message_notifications))
    .bind(i16::from(guild.explicit_content_filter))
    .bind(guild.afk_channel_id.map(|id| id.as_i64()))
    .bind(guild.afk_timeout as i32)
    .bind(guild.system_channel_id.map(|id| id.as_i64()))
    .bind(guild.system_channel_flags as i32)
    .bind(&guild.banner)
    .bind(&guild.splash)
    .bind(guild.prune_admin_only)
    .bind(guild.created_at)
    .execute(conn)
    .await
    .map_err(db)?;
    Ok(())
}

pub(super) async fn delete_guild(conn: &mut PgConnection, guild_id: Snowflake) -> AppResult<()> {
    sqlx::query("DELETE FROM user_guild_settings WHERE guild_id = $1")
        .bind(guild_id.as_i64())
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    sqlx::query("DELETE FROM guilds WHERE id = $1")
        .bind(guild_id.as_i64())
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}

pub(super) async fn upsert_role(conn: &mut PgConnection, role: &Role) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO roles (id, guild_id, name, color, hoist, icon, unicode_emoji, position, permissions, managed, \
             mentionable, bot_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, color = EXCLUDED.color, hoist = EXCLUDED.hoist, \
             icon = EXCLUDED.icon, unicode_emoji = EXCLUDED.unicode_emoji, position = EXCLUDED.position, \
             permissions = EXCLUDED.permissions, managed = EXCLUDED.managed, mentionable = EXCLUDED.mentionable, \
             bot_id = EXCLUDED.bot_id",
    )
    .bind(role.id.as_i64())
    .bind(role.guild_id.as_i64())
    .bind(&role.name)
    .bind(role.color as i32)
    .bind(role.hoist)
    .bind(&role.icon)
    .bind(&role.unicode_emoji)
    .bind(role.position)
    .bind(bits(role.permissions))
    .bind(role.managed)
    .bind(role.mentionable)
    .bind(role.bot_id.map(|id| id.as_i64()))
    .execute(conn)
    .await
    .map_err(db)?;
    Ok(())
}

pub(super) async fn delete_role(
    conn: &mut PgConnection,
    guild_id: Snowflake,
    role_id: Snowflake,
) -> AppResult<()> {
    sqlx::query("DELETE FROM roles WHERE guild_id = $1 AND id = $2")
        .bind(guild_id.as_i64())
        .bind(role_id.as_i64())
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}

pub(super) async fn insert_member(conn: &mut PgConnection, member: &Member) -> AppResult<()> {
    sqlx::query("INSERT INTO members (guild_id, user_id, joined_at) VALUES ($1, $2, $3)")
        .bind(member.guild_id.as_i64())
        .bind(member.user_id.as_i64())
        .bind(member.joined_at)
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    upsert_member(conn, member).await
}

pub(super) async fn upsert_member(conn: &mut PgConnection, member: &Member) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO members (guild_id, user_id, nick, avatar, joined_at, deaf, mute, communication_disabled_until, \
             pending) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
         ON CONFLICT (guild_id, user_id) DO UPDATE SET nick = EXCLUDED.nick, avatar = EXCLUDED.avatar, \
             deaf = EXCLUDED.deaf, mute = EXCLUDED.mute, \
             communication_disabled_until = EXCLUDED.communication_disabled_until, pending = EXCLUDED.pending",
    )
    .bind(member.guild_id.as_i64())
    .bind(member.user_id.as_i64())
    .bind(&member.nick)
    .bind(&member.avatar)
    .bind(member.joined_at)
    .bind(member.deaf)
    .bind(member.mute)
    .bind(member.communication_disabled_until)
    .bind(member.pending)
    .execute(&mut *conn)
    .await
    .map_err(db)?;

    sqlx::query("DELETE FROM member_roles WHERE guild_id = $1 AND user_id = $2")
        .bind(member.guild_id.as_i64())
        .bind(member.user_id.as_i64())
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    sqlx::query(
        "INSERT INTO member_roles (guild_id, user_id, role_id) \
         SELECT $1, $2, unnest($3::bigint[])",
    )
    .bind(member.guild_id.as_i64())
    .bind(member.user_id.as_i64())
    .bind(raw_ids(&member.role_ids))
    .execute(conn)
    .await
    .map_err(db)?;
    Ok(())
}

pub(super) async fn delete_member(
    conn: &mut PgConnection,
    guild_id: Snowflake,
    user_id: Snowflake,
) -> AppResult<()> {
    sqlx::query("DELETE FROM members WHERE guild_id = $1 AND user_id = $2")
        .bind(guild_id.as_i64())
        .bind(user_id.as_i64())
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}

pub(super) async fn insert_ban(
    conn: &mut PgConnection,
    guild_id: Snowflake,
    user_id: Snowflake,
    reason: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO bans (guild_id, user_id, reason) VALUES ($1, $2, $3) \
         ON CONFLICT (guild_id, user_id) DO UPDATE SET reason = EXCLUDED.reason",
    )
    .bind(guild_id.as_i64())
    .bind(user_id.as_i64())
    .bind(reason)
    .execute(conn)
    .await
    .map_err(db)?;
    Ok(())
}

pub(super) async fn delete_ban(
    conn: &mut PgConnection,
    guild_id: Snowflake,
    user_id: Snowflake,
) -> AppResult<()> {
    sqlx::query("DELETE FROM bans WHERE guild_id = $1 AND user_id = $2")
        .bind(guild_id.as_i64())
        .bind(user_id.as_i64())
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}

pub(super) async fn insert_invite(conn: &mut PgConnection, invite: &Invite) -> AppResult<()> {
    crate::postgres::vanity::reserve_ordinary(&mut *conn, &invite.code).await?;
    sqlx::query(
        "INSERT INTO invites (code, guild_id, channel_id, inviter_id, max_age, max_uses, uses, temporary, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(&invite.code)
    .bind(invite.guild_id.as_i64())
    .bind(invite.channel_id.as_i64())
    .bind(invite.inviter_id.map(|id| id.as_i64()))
    .bind(invite.max_age as i32)
    .bind(invite.max_uses as i32)
    .bind(invite.uses as i32)
    .bind(invite.temporary)
    .bind(invite.created_at)
    .execute(conn)
    .await
    .map_err(db)?;
    Ok(())
}

pub(super) async fn delete_invite(conn: &mut PgConnection, code: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM invites WHERE code = $1")
        .bind(code)
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}

/// Increments the use count atomically; fails when the invite is exhausted or gone.
pub(super) async fn consume_invite(conn: &mut PgConnection, code: &str) -> AppResult<()> {
    let result = sqlx::query(
        "UPDATE invites SET uses = uses + 1 WHERE code = $1 AND (max_uses = 0 OR uses < max_uses)",
    )
    .bind(code)
    .execute(conn)
    .await
    .map_err(db)?;
    if result.rows_affected() == 0 {
        return Err(AppError::Conflict {
            code: UNKNOWN_INVITE,
            message: "Unknown or exhausted invite.".into(),
        });
    }
    Ok(())
}

pub(super) async fn upsert_emoji(conn: &mut PgConnection, emoji: &Emoji) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO emojis (id, guild_id, name, creator_id, animated, available, require_colons, managed) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, creator_id = EXCLUDED.creator_id, \
             animated = EXCLUDED.animated, available = EXCLUDED.available, \
             require_colons = EXCLUDED.require_colons, managed = EXCLUDED.managed",
    )
    .bind(emoji.id.as_i64())
    .bind(emoji.guild_id.as_i64())
    .bind(&emoji.name)
    .bind(emoji.creator_id.map(|id| id.as_i64()))
    .bind(emoji.animated)
    .bind(emoji.available)
    .bind(emoji.require_colons)
    .bind(emoji.managed)
    .execute(&mut *conn)
    .await
    .map_err(db)?;

    sqlx::query("DELETE FROM emoji_roles WHERE emoji_id = $1")
        .bind(emoji.id.as_i64())
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    sqlx::query("INSERT INTO emoji_roles (emoji_id, role_id) SELECT $1, unnest($2::bigint[])")
        .bind(emoji.id.as_i64())
        .bind(raw_ids(&emoji.role_ids))
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}

pub(super) async fn delete_emoji(
    conn: &mut PgConnection,
    guild_id: Snowflake,
    emoji_id: Snowflake,
) -> AppResult<()> {
    sqlx::query("DELETE FROM emojis WHERE guild_id = $1 AND id = $2")
        .bind(guild_id.as_i64())
        .bind(emoji_id.as_i64())
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}

pub(super) async fn insert_audit(conn: &mut PgConnection, entry: &AuditLogEntry) -> AppResult<()> {
    let changes: Vec<serde_json::Value> = entry
        .changes
        .iter()
        .map(|c| {
            serde_json::json!({ "key": c.key, "old_value": c.old_value, "new_value": c.new_value })
        })
        .collect();
    sqlx::query(
        "INSERT INTO audit_log (id, guild_id, user_id, target_id, action, changes, reason, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(entry.id.as_i64())
    .bind(entry.guild_id.as_i64())
    .bind(entry.user_id.map(|id| id.as_i64()))
    .bind(entry.target_id.map(|id| id.as_i64()))
    .bind(entry.action.code() as i16)
    .bind(serde_json::Value::Array(changes))
    .bind(&entry.reason)
    .bind(entry.created_at)
    .execute(conn)
    .await
    .map_err(db)?;
    Ok(())
}
