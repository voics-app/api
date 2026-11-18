//! Row-locked timestamp reservations. No range is returned before commit.
use application::ports::IdReservations;
use application::{AppError, AppResult};
use async_trait::async_trait;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;
use voics_snowflake::{
    Reservation, ReservationRequest, EPOCH_MS, MAX_TIMESTAMP_MS, RESERVATION_MS,
};

#[derive(Clone)]
pub struct PgIdReservations {
    pool: PgPool,
}
impl PgIdReservations {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}
fn unavailable() -> AppError {
    AppError::Unavailable("ID reservation unavailable".into())
}
fn invalid() -> AppError {
    AppError::invalid(
        "id_reservation",
        "INVALID_RESERVATION",
        "Invalid reservation request",
    )
}
async fn database_now(conn: &mut PgConnection) -> AppResult<i64> {
    sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint")
        .fetch_one(conn)
        .await
        .map_err(|_| unavailable())
}
type Row = (i16, Option<Uuid>, i64, i64, i64);

#[async_trait]
impl IdReservations for PgIdReservations {
    async fn reserve(&self, request: ReservationRequest) -> AppResult<Reservation> {
        let (service, owner, worker, renew) = match request {
            ReservationRequest::Acquire {
                service,
                owner_id,
                preferred_worker,
            } => (service, owner_id, preferred_worker, false),
            ReservationRequest::Renew {
                service,
                owner_id,
                worker_id,
            } => (service, owner_id, worker_id, true),
        };
        if owner.is_nil() || !service.contains(worker) || (renew && !service.allocatable(worker)) {
            return Err(invalid());
        }
        let mut tx = self.pool.begin().await.map_err(|_| unavailable())?;
        sqlx::query("SET LOCAL lock_timeout = '2s'")
            .execute(&mut *tx)
            .await
            .map_err(|_| unavailable())?;
        sqlx::query("SET LOCAL statement_timeout = '5s'")
            .execute(&mut *tx)
            .await
            .map_err(|_| unavailable())?;
        let floor: i64 = sqlx::query_scalar(
            "SELECT not_before_ms FROM snowflake_allocation_guard WHERE singleton FOR SHARE",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| unavailable())?;
        let earliest = *service.workers().start() as i16;
        let latest = *service.workers().end() as i16;
        let row: Option<Row> = if renew {
            sqlx::query_as("SELECT worker_id, owner_id, start_ms, end_ms, issued_at_ms FROM snowflake_reservations WHERE worker_id=$1 FOR UPDATE")
                .bind(worker as i16).fetch_optional(&mut *tx).await.map_err(|_| unavailable())?
        } else {
            let own: Option<Row> = sqlx::query_as("SELECT worker_id, owner_id, start_ms, end_ms, issued_at_ms FROM snowflake_reservations WHERE owner_id=$1 AND worker_id BETWEEN $2 AND $3 FOR UPDATE")
                .bind(owner).bind(earliest).bind(latest).fetch_optional(&mut *tx).await.map_err(|_| unavailable())?;
            match own {
                Some(row) => Some(row),
                None => {
                    let now = database_now(&mut tx).await?;
                    sqlx::query_as("SELECT worker_id, owner_id, start_ms, end_ms, issued_at_ms FROM snowflake_reservations WHERE worker_id BETWEEN $1 AND $2 AND end_ms <= $3 ORDER BY (worker_id=$4) DESC, end_ms, worker_id FOR UPDATE SKIP LOCKED LIMIT 1")
                        .bind(earliest).bind(latest).bind(now).bind(worker as i16)
                        .fetch_optional(&mut *tx).await.map_err(|_| unavailable())?
                }
            }
        };
        let (worker_id, previous_owner, previous_start, previous_end, previous_issued) =
            row.ok_or_else(unavailable)?;
        // Sample after locking, never before a potentially long lock wait.
        let now = database_now(&mut tx).await?;
        if now < floor
            || now < EPOCH_MS as i64
            || now > (MAX_TIMESTAMP_MS - RESERVATION_MS) as i64
            || now < previous_issued
        {
            return Err(unavailable());
        }
        let same_owner = previous_owner == Some(owner);
        let (start, end) = if renew || same_owner {
            if !same_owner || now >= previous_end {
                return Err(unavailable());
            }
            (
                previous_start,
                if renew {
                    now + RESERVATION_MS as i64
                } else {
                    previous_end
                },
            )
        } else {
            if now < previous_end {
                return Err(unavailable());
            }
            (now.max(previous_end), now + RESERVATION_MS as i64)
        };
        let result = Reservation {
            worker_id: worker_id as u16,
            owner_id: owner,
            start_ms: start as u64,
            end_ms: end as u64,
            issued_at_ms: now as u64,
        };
        result.validate().map_err(|_| unavailable())?;
        sqlx::query("UPDATE snowflake_reservations SET owner_id=$1, start_ms=$2, end_ms=$3, issued_at_ms=$4 WHERE worker_id=$5")
            .bind(owner).bind(start).bind(end).bind(now).bind(worker_id)
            .execute(&mut *tx).await.map_err(|_| unavailable())?;
        tx.commit().await.map_err(|_| unavailable())?;
        Ok(result)
    }
}
