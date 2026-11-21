//! Deterministically race two real HTTP settings PATCHes against Postgres.
mod support;

use application::ports::UnitOfWork;
use application::write_set::{Mutation, WriteSet};
use application::AppResult;
use async_trait::async_trait;
use axum::http::{Method, StatusCode};
use interface::state::AppState;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use support::http::{call, request, send};
use support::{jwt_keys, ports, start_database};
use tokio::sync::Barrier;

struct RaceFirstTwo {
    inner: Arc<dyn UnitOfWork>,
    attempts: AtomicUsize,
    barrier: Barrier,
}
#[async_trait]
impl UnitOfWork for RaceFirstTwo {
    async fn commit(&self, writes: WriteSet) -> AppResult<()> {
        if writes
            .mutations
            .iter()
            .any(|m| matches!(m, Mutation::UpsertSettings(_)))
            && self.attempts.fetch_add(1, Ordering::SeqCst) < 2
        {
            // Both settings snapshots have already been read before either commits.
            self.barrier.wait().await;
        }
        self.inner.commit(writes).await
    }
}

#[tokio::test]
async fn concurrent_settings_patch_retries_and_emits_committed_snapshots() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let mut p = ports(&database.pool, &public, &private);
    let gate = Arc::new(RaceFirstTwo {
        inner: p.unit_of_work.clone(),
        attempts: AtomicUsize::new(0),
        barrier: Barrier::new(2),
    });
    p.unit_of_work = gate.clone();
    let users = p.users.clone();
    let app = interface::router(AppState {
        ports: p,
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let (status, user) = send(&app, request(Method::POST, "/api/v1/auth/register", None, Some(json!({"email":"settings-race@example.com","username":"settings_race","password":"a-long-enough-passphrase"})))).await;
    assert_eq!(status, StatusCode::CREATED);
    let token = user["access_token"].as_str().unwrap();
    let path = "/api/v1/users/@me/settings";
    let (a, b) = tokio::join!(
        call(
            &app,
            Method::PATCH,
            path,
            token,
            Some(json!({"theme":"light"}))
        ),
        call(
            &app,
            Method::PATCH,
            path,
            token,
            Some(json!({"preferences":{"hide_age_restricted_channels":true}}))
        )
    );
    assert_eq!(a.0, StatusCode::OK);
    assert_eq!(b.0, StatusCode::OK);
    assert!(
        gate.attempts.load(Ordering::SeqCst) >= 3,
        "one stale write must retry"
    );
    let (status, stored) = call(&app, Method::GET, path, token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored["theme"], "light");
    assert_eq!(
        stored["preferences"]["hide_age_restricted_channels"],
        json!(true)
    );
    let rows: Vec<(Value,)> = sqlx::query_as(
        "SELECT payload FROM outbox WHERE payload->>'type' = 'USER_SETTINGS_UPDATE' ORDER BY id",
    )
    .fetch_all(&database.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2, "the rejected attempt must not emit an event");
    let last = &rows[1].0["data"];
    assert_eq!(last["theme"], "light");
    assert_eq!(
        last["preferences"]["hide_age_restricted_channels"],
        json!(true)
    );

    // Two independent keys within the same sparse preferences object also merge on retry.
    gate.attempts.store(0, Ordering::SeqCst);
    let (a, b) = tokio::join!(
        call(
            &app,
            Method::PATCH,
            path,
            token,
            Some(json!({"preferences":{"reduce_motion":true}}))
        ),
        call(
            &app,
            Method::PATCH,
            path,
            token,
            Some(json!({"preferences":{"notify_sounds":false}}))
        )
    );
    assert_eq!(a.0, StatusCode::OK);
    assert_eq!(b.0, StatusCode::OK);
    assert!(gate.attempts.load(Ordering::SeqCst) >= 3);
    let (_, stored) = call(&app, Method::GET, path, token, None).await;
    assert_eq!(stored["preferences"]["reduce_motion"], json!(true));
    assert_eq!(stored["preferences"]["notify_sounds"], json!(false));
    let rows: Vec<(Value,)> = sqlx::query_as(
        "SELECT payload FROM outbox WHERE payload->>'type' = 'USER_SETTINGS_UPDATE' ORDER BY id",
    )
    .fetch_all(&database.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[3].0["data"]["preferences"], stored["preferences"]);

    // Defaults for a missing legacy row must not let two inserts both commit stale values.
    sqlx::query("DELETE FROM user_settings WHERE user_id = $1")
        .bind(user["user"]["id"].as_str().unwrap().parse::<i64>().unwrap())
        .execute(&database.pool)
        .await
        .unwrap();
    gate.attempts.store(0, Ordering::SeqCst);
    let (a, b) = tokio::join!(
        call(
            &app,
            Method::PATCH,
            path,
            token,
            Some(json!({"theme":"light"}))
        ),
        call(
            &app,
            Method::PATCH,
            path,
            token,
            Some(json!({"preferences":{"reduce_motion":true}}))
        )
    );
    assert_eq!(a.0, StatusCode::OK);
    assert_eq!(b.0, StatusCode::OK);
    assert!(gate.attempts.load(Ordering::SeqCst) >= 3);
    let (_, stored) = call(&app, Method::GET, path, token, None).await;
    assert_eq!(stored["theme"], "light");
    assert_eq!(stored["preferences"]["reduce_motion"], json!(true));

    // A non-CAS writer still advances the trigger, so a new writer using an old
    // snapshot is rejected and does not enqueue a stale event.
    let id = domain::Snowflake::new(user["user"]["id"].as_str().unwrap().parse().unwrap());
    let mut stale = users.settings(id).await.unwrap();
    sqlx::query("UPDATE user_settings SET theme = 'oled' WHERE user_id = $1")
        .bind(id.as_i64())
        .execute(&database.pool)
        .await
        .unwrap();
    stale
        .preferences
        .insert("notify_sounds".into(), json!(false));
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpsertSettings(stale.clone()))
        .emit(application::events::DomainEvent::UserSettingsUpdate(stale));
    assert!(matches!(
        gate.inner.commit(writes).await,
        Err(application::AppError::SettingsWriteConflict)
    ));
    let (_, after) = call(&app, Method::GET, path, token, None).await;
    assert_eq!(after["theme"], "oled");
    assert!(after["preferences"].get("notify_sounds").is_none());
    let rows: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM outbox WHERE payload->>'type' = 'USER_SETTINGS_UPDATE'",
    )
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(rows.0, 6, "the stale attempt must not enqueue an event");

    // Independent server overrides in the same nested privacy map must both survive.
    gate.attempts.store(0, Ordering::SeqCst);
    let (a, b) = tokio::join!(
        call(
            &app,
            Method::PATCH,
            path,
            token,
            Some(json!({"preferences":{"guild_dm_restrictions":{"1001":true}}}))
        ),
        call(
            &app,
            Method::PATCH,
            path,
            token,
            Some(json!({"preferences":{"guild_dm_restrictions":{"1002":false}}}))
        )
    );
    assert_eq!(a.0, StatusCode::OK);
    assert_eq!(b.0, StatusCode::OK);
    assert!(gate.attempts.load(Ordering::SeqCst) >= 3);
    let (_, stored) = call(&app, Method::GET, path, token, None).await;
    assert_eq!(
        stored["preferences"]["guild_dm_restrictions"],
        json!({"1001":true,"1002":false})
    );
    let rows: Vec<(Value,)> = sqlx::query_as(
        "SELECT payload FROM outbox WHERE payload->>'type' = 'USER_SETTINGS_UPDATE' ORDER BY id",
    )
    .fetch_all(&database.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 8);
    assert_eq!(
        rows[7].0["data"]["preferences"]["guild_dm_restrictions"],
        json!({"1001":true,"1002":false})
    );
}
