use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::FromRow;

use application::ports::DeviceRepository;
use application::{AppError, AppResult};
use domain::device::{Device, IDENTITY_KEY_LENGTH};
use domain::Snowflake;

use super::convert::sf;
use super::error::db;
use super::PgStore;

#[derive(FromRow)]
struct DeviceRow {
    id: i64,
    user_id: i64,
    name: String,
    identity_key: Vec<u8>,
    created_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
}

impl TryFrom<DeviceRow> for Device {
    type Error = AppError;

    fn try_from(row: DeviceRow) -> Result<Self, AppError> {
        let identity_key: [u8; IDENTITY_KEY_LENGTH] = row
            .identity_key
            .try_into()
            .map_err(|_| AppError::internal("stored identity key has the wrong length"))?;
        Ok(Device {
            id: sf(row.id),
            user_id: sf(row.user_id),
            name: row.name,
            identity_key,
            created_at: row.created_at,
            last_seen_at: row.last_seen_at,
        })
    }
}

const DEVICE_COLUMNS: &str = "id, user_id, name, identity_key, created_at, last_seen_at";

#[async_trait]
impl DeviceRepository for PgStore {
    async fn find(&self, id: Snowflake) -> AppResult<Option<Device>> {
        let sql = format!("SELECT {DEVICE_COLUMNS} FROM devices WHERE id = $1");
        let row: Option<DeviceRow> = sqlx::query_as(&sql)
            .bind(id.as_i64())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        row.map(Device::try_from).transpose()
    }

    async fn list_for_user(&self, user_id: Snowflake) -> AppResult<Vec<Device>> {
        let sql = format!("SELECT {DEVICE_COLUMNS} FROM devices WHERE user_id = $1 ORDER BY id");
        let rows: Vec<DeviceRow> = sqlx::query_as(&sql)
            .bind(user_id.as_i64())
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        rows.into_iter().map(Device::try_from).collect()
    }

    /// Bumped at most hourly: the idle window is measured in days, and refreshes are frequent.
    async fn touch(&self, device_id: Snowflake, now: DateTime<Utc>) -> AppResult<()> {
        sqlx::query(
            "UPDATE devices SET last_seen_at = $2 \
             WHERE id = $1 AND last_seen_at < $2 - interval '1 hour'",
        )
        .bind(device_id.as_i64())
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn list_stale(&self, cutoff: DateTime<Utc>, limit: u32) -> AppResult<Vec<Device>> {
        let sql = format!(
            "SELECT {DEVICE_COLUMNS} FROM devices WHERE last_seen_at < $1 ORDER BY last_seen_at LIMIT $2"
        );
        let rows: Vec<DeviceRow> = sqlx::query_as(&sql)
            .bind(cutoff)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        rows.into_iter().map(Device::try_from).collect()
    }

    async fn add_key_packages(&self, device_id: Snowflake, packages: &[Vec<u8>]) -> AppResult<()> {
        sqlx::query("INSERT INTO key_packages (device_id, package) SELECT $1, unnest($2::bytea[])")
            .bind(device_id.as_i64())
            .bind(packages)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    /// `DELETE ... RETURNING` over a `SKIP LOCKED` pick: concurrent claims
    /// never see the same row, and a claimed package is gone for good.
    async fn claim_key_package(&self, device_id: Snowflake) -> AppResult<Option<Vec<u8>>> {
        let row: Option<(Vec<u8>,)> = sqlx::query_as(
            "DELETE FROM key_packages WHERE id = ( \
                 SELECT id FROM key_packages WHERE device_id = $1 ORDER BY id LIMIT 1 FOR UPDATE SKIP LOCKED \
             ) RETURNING package",
        )
        .bind(device_id.as_i64())
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row.map(|(package,)| package))
    }
}
