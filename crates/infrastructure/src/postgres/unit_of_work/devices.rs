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

/// Key packages go with the device (`ON DELETE CASCADE`); sessions bound to
/// it are unbound (`ON DELETE SET NULL`).
pub(super) async fn delete_device(
    conn: &mut PgConnection,
    user_id: Snowflake,
    device_id: Snowflake,
) -> AppResult<()> {
    sqlx::query("DELETE FROM devices WHERE id = $1 AND user_id = $2")
        .bind(device_id.as_i64())
        .bind(user_id.as_i64())
        .execute(conn)
        .await
        .map_err(db)?;
    Ok(())
}
