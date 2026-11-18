//! Writes for devices and their identity keys.

use sqlx::PgConnection;

use application::AppResult;
use domain::device::Device;
use domain::Snowflake;

use crate::postgres::error::db;

pub(super) async fn insert_device(conn: &mut PgConnection, device: &Device) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO devices (id, user_id, name, identity_key, created_at, last_seen_at) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(device.id.as_i64())
    .bind(device.user_id.as_i64())
    .bind(&device.name)
    .bind(device.identity_key.as_slice())
    .bind(device.created_at)
    .bind(device.last_seen_at)
    .execute(conn)
    .await
    .map_err(db)?;
    Ok(())
}

/// Notification and PushKit tokens go with all sessions bound to the device,
/// regardless of registration order. Sessions remain signed in but unbound;
/// key packages and legacy device-linked push rows also cascade on deletion.
pub(super) async fn delete_device(
    conn: &mut PgConnection,
    user_id: Snowflake,
    device_id: Snowflake,
) -> AppResult<()> {
    super::super::push_tokens::lock_user(conn, user_id).await?;
    let sessions: Vec<uuid::Uuid> = sqlx::query_scalar(
        "UPDATE sessions SET device_id = NULL WHERE device_id = $1 AND user_id = $2 RETURNING id",
    )
    .bind(device_id.as_i64())
    .bind(user_id.as_i64())
    .fetch_all(&mut *conn)
    .await
    .map_err(db)?;
    for table in ["push_tokens", "voip_tokens"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE session_id = ANY($1)"))
            .bind(&sessions)
            .execute(&mut *conn)
            .await
            .map_err(db)?;
    }
    sqlx::query("DELETE FROM devices WHERE id = $1 AND user_id = $2")
        .bind(device_id.as_i64())
        .bind(user_id.as_i64())
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}
