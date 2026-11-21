//! Writes for applications, bot tokens, commands, interactions and webhooks.

use sqlx::PgConnection;

use application::AppResult;
use domain::application::{
    Application, ApplicationCommand, Interaction, InteractionState, Webhook,
};
use domain::Snowflake;

use crate::postgres::convert::bits;
use crate::postgres::error::db;

pub(super) async fn upsert_application(
    conn: &mut PgConnection,
    app: &Application,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO applications (id, name, description, icon, owner_id, bot_user_id, bot_public, \
             bot_require_code_grant, verify_key, flags, interactions_endpoint_url, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, description = EXCLUDED.description, \
             icon = EXCLUDED.icon, owner_id = EXCLUDED.owner_id, bot_public = EXCLUDED.bot_public, \
             bot_require_code_grant = EXCLUDED.bot_require_code_grant, verify_key = EXCLUDED.verify_key, \
             flags = EXCLUDED.flags, interactions_endpoint_url = EXCLUDED.interactions_endpoint_url",
    )
    .bind(app.id.as_i64())
    .bind(&app.name)
    .bind(&app.description)
    .bind(&app.icon)
    .bind(app.owner_id.as_i64())
    .bind(app.bot_user_id.as_i64())
    .bind(app.bot_public)
    .bind(app.bot_require_code_grant)
    .bind(&app.verify_key)
    .bind(app.flags as i64)
    .bind(&app.interactions_endpoint_url)
    .bind(app.created_at)
    .execute(conn)
    .await
    .map_err(db)?;
    Ok(())
}

pub(super) async fn set_bot_token(
    conn: &mut PgConnection,
    application_id: Snowflake,
    bot_user_id: Snowflake,
    token_hash: &str,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO bot_tokens (application_id, bot_user_id, token_hash) VALUES ($1, $2, $3) \
         ON CONFLICT (application_id) DO UPDATE SET bot_user_id = EXCLUDED.bot_user_id, \
             token_hash = EXCLUDED.token_hash, created_at = now()",
    )
    .bind(application_id.as_i64())
    .bind(bot_user_id.as_i64())
    .bind(token_hash)
    .execute(conn)
    .await
    .map_err(db)?;
    Ok(())
}

pub(super) async fn upsert_command(
    conn: &mut PgConnection,
    command: &ApplicationCommand,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO application_commands (id, application_id, guild_id, name, description, kind, options, \
             default_member_permissions, dm_permission, nsfw, version) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, description = EXCLUDED.description, \
             kind = EXCLUDED.kind, options = EXCLUDED.options, \
             default_member_permissions = EXCLUDED.default_member_permissions, \
             dm_permission = EXCLUDED.dm_permission, nsfw = EXCLUDED.nsfw, version = EXCLUDED.version",
    )
    .bind(command.id.as_i64())
    .bind(command.application_id.as_i64())
    .bind(command.guild_id.map(|id| id.as_i64()))
    .bind(&command.name)
    .bind(&command.description)
    .bind(i16::from(command.kind.code()))
    .bind(&command.options)
    .bind(command.default_member_permissions.map(bits))
    .bind(command.dm_permission)
    .bind(command.nsfw)
    .bind(command.version.as_i64())
    .execute(conn)
    .await
    .map_err(db)?;
    Ok(())
}

pub(super) async fn delete_command(
    conn: &mut PgConnection,
    application_id: Snowflake,
    command_id: Snowflake,
) -> AppResult<()> {
    sqlx::query("DELETE FROM application_commands WHERE application_id = $1 AND id = $2")
        .bind(application_id.as_i64())
        .bind(command_id.as_i64())
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}

pub(super) async fn replace_commands(
    conn: &mut PgConnection,
    application_id: Snowflake,
    guild_id: Option<Snowflake>,
    commands: &[ApplicationCommand],
) -> AppResult<()> {
    sqlx::query("DELETE FROM application_commands WHERE application_id = $1 AND guild_id IS NOT DISTINCT FROM $2")
        .bind(application_id.as_i64())
        .bind(guild_id.map(|id| id.as_i64()))
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    for command in commands {
        upsert_command(&mut *conn, command).await?;
    }
    Ok(())
}

pub(super) async fn insert_interaction(
    conn: &mut PgConnection,
    interaction: &Interaction,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO interactions (id, application_id, kind, guild_id, channel_id, user_id, token_hash, data, state, \
             original_message_id, source_message_id, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(interaction.id.as_i64())
    .bind(interaction.application_id.as_i64())
    .bind(i16::from(interaction.kind.code()))
    .bind(interaction.guild_id.map(|id| id.as_i64()))
    .bind(interaction.channel_id.as_i64())
    .bind(interaction.user_id.as_i64())
    .bind(&interaction.token_hash)
    .bind(&interaction.data)
    .bind(interaction.state.as_str())
    .bind(interaction.original_message_id.map(|id| id.as_i64()))
    .bind(interaction.source_message_id.map(|id| id.as_i64()))
    .bind(interaction.created_at)
    .execute(conn)
    .await
    .map_err(db)?;
    Ok(())
}

pub(super) async fn update_interaction(
    conn: &mut PgConnection,
    id: Snowflake,
    state: InteractionState,
    original_message_id: Option<Snowflake>,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE interactions SET state = $2, original_message_id = COALESCE($3, original_message_id) WHERE id = $1",
    )
    .bind(id.as_i64())
    .bind(state.as_str())
    .bind(original_message_id.map(|id| id.as_i64()))
    .execute(conn)
    .await
    .map_err(db)?;
    Ok(())
}

pub(super) async fn insert_webhook(conn: &mut PgConnection, webhook: &Webhook) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO webhooks (id, kind, guild_id, channel_id, creator_id, name, avatar, application_id, token_hash) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(webhook.id.as_i64())
    .bind(i16::from(webhook.kind.code()))
    .bind(webhook.guild_id.as_i64())
    .bind(webhook.channel_id.as_i64())
    .bind(webhook.creator_id.map(|id| id.as_i64()))
    .bind(&webhook.name)
    .bind(&webhook.avatar)
    .bind(webhook.application_id.map(|id| id.as_i64()))
    .bind(&webhook.token_hash)
    .execute(conn)
    .await
    .map_err(db)?;
    Ok(())
}

pub(super) async fn update_webhook(
    conn: &mut PgConnection,
    id: Snowflake,
    patch: &application::write_set::WebhookMetadataPatch,
) -> AppResult<()> {
    let result = sqlx::query(
        "UPDATE webhooks SET name = COALESCE($2, name), \
         avatar = CASE WHEN $3 THEN $4 ELSE avatar END, \
         channel_id = COALESCE($5, channel_id) WHERE id = $1",
    )
    .bind(id.as_i64())
    .bind(&patch.name)
    .bind(patch.avatar.is_some())
    .bind(patch.avatar.as_ref().and_then(|avatar| avatar.as_deref()))
    .bind(patch.channel_id.map(|id| id.as_i64()))
    .execute(conn)
    .await
    .map_err(db)?;
    if result.rows_affected() == 0 {
        return Err(application::AppError::NotFound(
            application::Resource::Webhook,
        ));
    }
    Ok(())
}

pub(super) async fn delete_webhook(conn: &mut PgConnection, id: Snowflake) -> AppResult<()> {
    sqlx::query("DELETE FROM webhooks WHERE id = $1")
        .bind(id.as_i64())
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}

pub(super) async fn set_webhook_token(
    conn: &mut PgConnection,
    id: Snowflake,
    hash: &str,
) -> AppResult<()> {
    let result = sqlx::query("UPDATE webhooks SET token_hash = $2 WHERE id = $1 AND kind = 1")
        .bind(id.as_i64())
        .bind(hash)
        .execute(conn)
        .await
        .map_err(db)?;
    if result.rows_affected() == 0 {
        return Err(application::AppError::NotFound(
            application::Resource::Webhook,
        ));
    }
    Ok(())
}
