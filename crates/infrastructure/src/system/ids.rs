//! Issuance from committed PostgreSQL ranges, with no static fallback.
use application::ports::{IdGenerator, IdReservations};
use application::{AppError, AppResult};
use domain::Snowflake;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;
use voics_snowflake::{LeasedIds, ReservationRequest, Service, RENEW_EVERY};

pub struct SnowflakeIds {
    issuer: LeasedIds,
}

impl SnowflakeIds {
    pub async fn start(
        allocator: Arc<dyn IdReservations>,
        preferred_worker: u16,
    ) -> AppResult<Arc<Self>> {
        let requested_at = Instant::now();
        let owner_id = Uuid::new_v4();
        let reservation = allocator
            .reserve(ReservationRequest::Acquire {
                service: Service::Api,
                owner_id,
                preferred_worker,
            })
            .await?;
        if reservation.owner_id != owner_id || !Service::Api.allocatable(reservation.worker_id) {
            return Err(unavailable(voics_snowflake::IdError::InvalidReservation));
        }
        let ids = Arc::new(Self {
            issuer: LeasedIds::new(reservation, requested_at).map_err(unavailable)?,
        });
        let weak = Arc::downgrade(&ids);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(RENEW_EVERY).await;
                let Some(ids) = weak.upgrade() else {
                    return;
                };
                let Ok(current) = ids.issuer.reservation() else {
                    return;
                };
                let requested_at = Instant::now();
                let result = allocator
                    .reserve(ReservationRequest::Renew {
                        service: Service::Api,
                        owner_id: current.owner_id,
                        worker_id: current.worker_id,
                    })
                    .await;
                match result {
                    Ok(next) => match ids.issuer.renew(next, requested_at) {
                        Ok(()) => {}
                        Err(voics_snowflake::IdError::Expired) => {
                            tracing::warn!("ID reservation expired; restart required");
                            return;
                        }
                        Err(_) => tracing::warn!("ID renewal rejected; committed bound unchanged"),
                    },
                    Err(_) => tracing::warn!("ID renewal unavailable; committed bound unchanged"),
                }
            }
        });
        tracing::info!(worker_id = reservation.worker_id, "reserved API ID worker");
        Ok(ids)
    }
}

fn unavailable(error: voics_snowflake::IdError) -> AppError {
    AppError::Unavailable(error.to_string())
}
impl IdGenerator for SnowflakeIds {
    fn next_id(&self) -> AppResult<Snowflake> {
        self.issuer
            .next_id()
            .map(Snowflake::new)
            .map_err(unavailable)
    }
    fn is_ready(&self) -> bool {
        self.issuer.is_ready()
    }
}
