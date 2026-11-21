//! Real PostgreSQL concurrency/fencing, plus the public failure surface.
mod support;
use application::ports::{IdGenerator, IdReservations};
use application::{AppError, AppResult};
use axum::http::{Method, StatusCode};
use infrastructure::postgres::PgIdReservations;
use infrastructure::system::ids::SnowflakeIds;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use support::http::{request, send, INTERNAL_TOKEN};
use uuid::Uuid;
use voics_snowflake::{Generator, Reservation, ReservationRequest, Service, RESERVATION_MS};

fn acquire(service: Service, preferred_worker: u16) -> ReservationRequest {
    ReservationRequest::Acquire {
        service,
        owner_id: Uuid::new_v4(),
        preferred_worker,
    }
}
fn renew(reservation: Reservation) -> ReservationRequest {
    ReservationRequest::Renew {
        service: if reservation.worker_id < 64 {
            Service::Api
        } else {
            Service::Messages
        },
        owner_id: reservation.owner_id,
        worker_id: reservation.worker_id,
    }
}

#[tokio::test]
async fn concurrent_replicas_exclude_legacy_workers_and_never_repeat_ids() {
    let database = support::start_database().await;
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..16 {
        // Independent adapters, same configured preference, same database.
        let allocator = PgIdReservations::new(database.pool.clone());
        tasks.spawn(async move { allocator.reserve(acquire(Service::Api, 0)).await.unwrap() });
    }
    let mut leases = Vec::new();
    while let Some(result) = tasks.join_next().await {
        leases.push(result.unwrap());
    }
    let workers: HashSet<_> = leases.iter().map(|lease| lease.worker_id).collect();
    assert_eq!(workers.len(), 16);
    assert!(!workers.contains(&0));
    let now = leases.iter().map(|lease| lease.start_ms).max().unwrap();
    let mut ids = HashSet::new();
    for lease in &leases {
        let mut generator = Generator::new(*lease).unwrap();
        for _ in 0..1000 {
            assert!(ids.insert(generator.next(now, Duration::ZERO).unwrap()));
        }
    }
    assert_eq!(ids.len(), 16_000);
    let allocator = PgIdReservations::new(database.pool.clone());
    let renewed = allocator.reserve(renew(leases[0])).await.unwrap();
    assert_eq!(renewed.start_ms, leases[0].start_ms);
    assert!(renewed.end_ms >= leases[0].end_ms);
    let messages = allocator
        .reserve(acquire(Service::Messages, 64))
        .await
        .unwrap();
    assert_eq!(messages.worker_id, 65);
    assert!(allocator
        .reserve(ReservationRequest::Renew {
            service: Service::Messages,
            owner_id: messages.owner_id,
            worker_id: 64
        })
        .await
        .is_err());
    for _ in 16..63 {
        allocator.reserve(acquire(Service::Api, 0)).await.unwrap();
    }
    assert!(matches!(
        allocator.reserve(acquire(Service::Api, 0)).await,
        Err(AppError::Unavailable(_))
    ));
}

#[tokio::test]
async fn reassignment_and_restore_floor_fence_old_and_restored_owners() {
    let database = support::start_database().await;
    let allocator = PgIdReservations::new(database.pool.clone());
    let now: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint")
            .fetch_one(&database.pool)
            .await
            .unwrap();
    let now = now as u64;
    let old = Reservation {
        worker_id: 1,
        owner_id: Uuid::new_v4(),
        start_ms: now - 200,
        end_ms: now - 100,
        issued_at_ms: now - 200,
    };
    sqlx::query("UPDATE snowflake_reservations SET owner_id=$1,start_ms=$2,end_ms=$3,issued_at_ms=$2 WHERE worker_id=1")
        .bind(old.owner_id).bind(old.start_ms as i64).bind(old.end_ms as i64).execute(&database.pool).await.unwrap();
    let new = allocator.reserve(acquire(Service::Api, 1)).await.unwrap();
    assert_eq!(new.worker_id, 1);
    assert!(new.start_ms >= old.end_ms);
    assert!(allocator.reserve(renew(old)).await.is_err());
    let mut previous = Generator::new(old).unwrap();
    let last = previous
        .next(old.end_ms - 1, Duration::from_millis(99))
        .unwrap();
    let mut current = Generator::new(new).unwrap();
    assert!(current.next(new.start_ms, Duration::ZERO).unwrap() > last);
    assert!(
        previous
            .next(old.end_ms - 1, Duration::from_millis(100))
            .is_err(),
        "resumed process fails even if wall clock rolled back"
    );

    let floor = new.end_ms + RESERVATION_MS;
    sqlx::query("UPDATE snowflake_allocation_guard SET not_before_ms=$1")
        .bind(floor as i64)
        .execute(&database.pool)
        .await
        .unwrap();
    assert!(allocator
        .reserve(acquire(Service::Messages, 64))
        .await
        .is_err());
    assert!(allocator.reserve(renew(new)).await.is_err());
    // Failed renewal did not extend the committed high-water mark.
    let stored: i64 =
        sqlx::query_scalar("SELECT end_ms FROM snowflake_reservations WHERE worker_id=1")
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(stored, new.end_ms as i64);
}

#[tokio::test]
async fn reservations_require_internal_auth_and_generation_failure_writes_nothing() {
    let database = support::start_database().await;
    let (private, public) = support::jwt_keys();
    let mut ports = support::ports(&database.pool, &public, &private);
    struct Expired;
    impl IdGenerator for Expired {
        fn next_id(&self) -> AppResult<domain::Snowflake> {
            Err(AppError::Unavailable("ID reservation expired".into()))
        }
        fn is_ready(&self) -> bool {
            false
        }
    }
    ports.ids = Arc::new(Expired);
    let app = interface::router(interface::state::AppState {
        ports,
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: Arc::new(support::AllowAll),
        rate_policy: Default::default(),
    });
    let body = serde_json::to_value(acquire(Service::Messages, 64)).unwrap();
    let (status, _) = send(
        &app,
        request(
            Method::POST,
            "/internal/v1/id-reservations",
            None,
            Some(body.clone()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let mut authenticated = request(
        Method::POST,
        "/internal/v1/id-reservations",
        None,
        Some(body),
    );
    authenticated
        .headers_mut()
        .insert("x-internal-token", INTERNAL_TOKEN.parse().unwrap());
    let (status, lease) = send(&app, authenticated).await;
    assert_eq!(status, StatusCode::OK, "{lease}");
    assert_eq!(lease["worker_id"], 65);
    let mut invalid = request(
        Method::POST,
        "/internal/v1/id-reservations",
        None,
        Some(serde_json::to_value(acquire(Service::Api, 0)).unwrap()),
    );
    invalid
        .headers_mut()
        .insert("x-internal-token", INTERNAL_TOKEN.parse().unwrap());
    let (status, _) = send(&app, invalid).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = send(&app, request(Method::GET, "/readyz", None, None)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let (status,body)=send(&app,request(Method::POST,"/api/v1/auth/register",None,Some(json!({"email":"allocation@example.test","username":"allocation","password":"a-long-enough-passphrase"})))).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    assert_eq!(users, 0);
    let outbox: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    assert_eq!(outbox, 0);
}

#[tokio::test]
async fn runtime_replicas_use_distinct_workers_and_short_reservations_expire() {
    let database = support::start_database().await;
    let allocator = Arc::new(PgIdReservations::new(database.pool.clone()));
    let mut ids = HashSet::new();
    let mut workers = HashSet::new();
    for _ in 0..3 {
        let issuer = SnowflakeIds::start(allocator.clone(), 0).await.unwrap();
        // Wait for a small host/database clock offset without exceeding the reservation.
        for _ in 0..100 {
            if issuer.is_ready() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(issuer.is_ready());
        for _ in 0..1000 {
            let id = issuer.next_id().unwrap().value();
            workers.insert((id >> 12) & 1023);
            assert!(ids.insert(id));
        }
    }
    assert_eq!(workers.len(), 3);
    assert!(!workers.contains(&0));

    struct ShortReservation;
    #[async_trait::async_trait]
    impl IdReservations for ShortReservation {
        async fn reserve(&self, request: ReservationRequest) -> AppResult<Reservation> {
            let ReservationRequest::Acquire { owner_id, .. } = request else {
                return Err(AppError::Unavailable("test renewal outage".into()));
            };
            let now = voics_snowflake::unix_ms();
            Ok(Reservation {
                worker_id: 1,
                owner_id,
                start_ms: now,
                end_ms: now + 100,
                issued_at_ms: now,
            })
        }
    }
    let issuer = SnowflakeIds::start(Arc::new(ShortReservation), 0)
        .await
        .unwrap();
    assert!(issuer.next_id().is_ok());
    tokio::time::sleep(Duration::from_millis(110)).await;
    assert!(!issuer.is_ready());
    assert!(matches!(issuer.next_id(), Err(AppError::Unavailable(_))));
}

#[tokio::test]
async fn lock_wait_samples_current_database_time_and_clock_rollback_cannot_renew() {
    let database = support::start_database().await;
    let allocator = Arc::new(PgIdReservations::new(database.pool.clone()));
    let first = allocator.reserve(acquire(Service::Api, 0)).await.unwrap();
    let mut lock = database.pool.begin().await.unwrap();
    sqlx::query("SELECT worker_id FROM snowflake_reservations WHERE worker_id=$1 FOR UPDATE")
        .bind(first.worker_id as i16)
        .fetch_one(&mut *lock)
        .await
        .unwrap();
    let other = allocator.clone();
    let waiting = tokio::spawn(async move { other.reserve(renew(first)).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let before_release: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint")
            .fetch_one(&mut *lock)
            .await
            .unwrap();
    lock.commit().await.unwrap();
    let renewed = waiting.await.unwrap().unwrap();
    assert!(renewed.issued_at_ms >= before_release as u64);
    // Simulate the durable observation left by a clock that subsequently moved back.
    let future_issued = renewed.issued_at_ms + 60_000;
    sqlx::query("UPDATE snowflake_reservations SET issued_at_ms=$1,end_ms=$2 WHERE worker_id=$3")
        .bind(future_issued as i64)
        .bind((future_issued + RESERVATION_MS) as i64)
        .bind(first.worker_id as i16)
        .execute(&database.pool)
        .await
        .unwrap();
    assert!(allocator.reserve(renew(renewed)).await.is_err());
}

#[tokio::test]
async fn actual_restore_script_preserves_live_and_restored_high_watermarks() {
    let database = support::start_database().await;
    let allocator = PgIdReservations::new(database.pool.clone());
    let old = allocator.reserve(acquire(Service::Api, 0)).await.unwrap();
    // An older backup can have a smaller end than the captured pre-restore database.
    let captured_end = old.end_ms + 60_000;
    let script = include_str!("../../../scripts/restore-id-barrier.sql")
        .replace(":'previous_end_ms'", &captured_end.to_string());
    sqlx::raw_sql(&script)
        .execute(&database.pool)
        .await
        .unwrap();
    let floor: i64 = sqlx::query_scalar("SELECT not_before_ms FROM snowflake_allocation_guard")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    assert!(floor as u64 >= captured_end);
    let (owners, earliest): (i64, i64) =
        sqlx::query_as("SELECT count(owner_id),min(end_ms) FROM snowflake_reservations")
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(owners, 0);
    assert!(earliest >= floor);
    assert!(allocator.reserve(renew(old)).await.is_err());
    assert!(allocator.reserve(acquire(Service::Api, 0)).await.is_err());
    assert!(allocator
        .reserve(acquire(Service::Messages, 64))
        .await
        .is_err());
    // Reapplying with an older capture cannot lower the barrier.
    let script = include_str!("../../../scripts/restore-id-barrier.sql")
        .replace(":'previous_end_ms'", &old.end_ms.to_string());
    sqlx::raw_sql(&script)
        .execute(&database.pool)
        .await
        .unwrap();
    let second: i64 = sqlx::query_scalar("SELECT not_before_ms FROM snowflake_allocation_guard")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    assert!(second >= floor);
}

#[tokio::test]
async fn restore_script_rejects_bad_input_or_missing_guard_and_rolls_back_all_changes() {
    let database = support::start_database().await;
    let allocator = PgIdReservations::new(database.pool.clone());
    let old = allocator.reserve(acquire(Service::Api, 0)).await.unwrap();
    let original = include_str!("../../../scripts/restore-id-barrier.sql");
    let mut scripts = Vec::new();
    for value in [
        "-1".to_owned(),
        "0".to_owned(),
        (voics_snowflake::EPOCH_MS - 1).to_string(),
        (voics_snowflake::MAX_TIMESTAMP_MS + 1).to_string(),
        "'not-an-integer'".to_owned(),
    ] {
        scripts.push(original.replace(":'previous_end_ms'", &value));
    }
    // Unset psql input remains unresolved; it must not become a default watermark.
    scripts.push(original.to_owned());
    // Failure after both UPDATEs must roll owner and guard back atomically.
    scripts.push(
        original
            .replace(":'previous_end_ms'", &old.end_ms.to_string())
            .replace("COMMIT;", "SELECT 1/0;\nCOMMIT;"),
    );
    for script in scripts {
        let mut connection = database.pool.acquire().await.unwrap();
        assert!(sqlx::raw_sql(&script)
            .execute(&mut *connection)
            .await
            .is_err());
        sqlx::query("ROLLBACK")
            .execute(&mut *connection)
            .await
            .unwrap();
        let floor: i64 = sqlx::query_scalar("SELECT not_before_ms FROM snowflake_allocation_guard")
            .fetch_one(&mut *connection)
            .await
            .unwrap();
        assert_eq!(floor, 0);
        let row: (Option<Uuid>, i64) =
            sqlx::query_as("SELECT owner_id,end_ms FROM snowflake_reservations WHERE worker_id=$1")
                .bind(old.worker_id as i16)
                .fetch_one(&mut *connection)
                .await
                .unwrap();
        assert_eq!(row, (Some(old.owner_id), old.end_ms as i64));
    }
    sqlx::query("DELETE FROM snowflake_allocation_guard")
        .execute(&database.pool)
        .await
        .unwrap();
    let script = original.replace(":'previous_end_ms'", &old.end_ms.to_string());
    let mut connection = database.pool.acquire().await.unwrap();
    assert!(sqlx::raw_sql(&script)
        .execute(&mut *connection)
        .await
        .is_err());
    sqlx::query("ROLLBACK")
        .execute(&mut *connection)
        .await
        .unwrap();
    let owner: Option<Uuid> =
        sqlx::query_scalar("SELECT owner_id FROM snowflake_reservations WHERE worker_id=$1")
            .bind(old.worker_id as i16)
            .fetch_one(&mut *connection)
            .await
            .unwrap();
    assert_eq!(owner, Some(old.owner_id));
    drop(connection);
    assert!(allocator.reserve(acquire(Service::Api, 0)).await.is_err());
}
