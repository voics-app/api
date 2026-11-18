//! Device-bound push cleanup must work whether push or E2EE registration finishes first.

mod support;

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use serde_json::json;

use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database, Database};

const PASSWORD: &str = "push-lifecycle-test-password";
const KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const PHONE_PUSH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PHONE_VOIP: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const LAPTOP_PUSH: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

fn app(database: &Database) -> (axum::Router, application::Ports) {
    let (private, public) = jwt_keys();
    let ports = ports(&database.pool, &public, &private);
    let app = interface::router(AppState {
        ports: ports.clone(),
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    (app, ports)
}

async fn account(app: &axum::Router) -> String {
    let (status, body) = send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({"email":"push-lifecycle@example.invalid", "username":"push_lifecycle", "password":PASSWORD})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    body["access_token"]
        .as_str()
        .expect("access token")
        .to_owned()
}

async fn second_session(app: &axum::Router) -> String {
    let (status, body) = send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/login",
            None,
            Some(json!({"email":"push-lifecycle@example.invalid", "password":PASSWORD})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body["access_token"]
        .as_str()
        .expect("access token")
        .to_owned()
}

async fn push(app: &axum::Router, access: &str, token: &str) {
    assert_eq!(
        call(
            app,
            Method::POST,
            "/api/v1/users/@me/devices/push",
            access,
            Some(json!({"platform":"apns", "token":token}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT,
    );
}

async fn phone_tokens(app: &axum::Router, access: &str) {
    push(app, access, PHONE_PUSH).await;
    assert_eq!(
        call(
            app,
            Method::POST,
            "/api/v1/users/@me/devices/voip",
            access,
            Some(json!({"token":PHONE_VOIP}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT,
    );
}

async fn device(app: &axum::Router, access: &str) -> String {
    let (status, body) = call(
        app,
        Method::POST,
        "/api/v1/devices",
        access,
        Some(json!({"name":"Phone", "identity_key_b64":KEY})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    body["device_id"].as_str().expect("device id").to_owned()
}

async fn assert_only_laptop_remains(database: &Database) {
    let push: Vec<String> = sqlx::query_scalar("SELECT token FROM push_tokens ORDER BY token")
        .fetch_all(&database.pool)
        .await
        .expect("push tokens");
    assert_eq!(
        push,
        vec![LAPTOP_PUSH],
        "the removed device's notification token must leave, even if registered before the device"
    );
    let voip: i64 = sqlx::query_scalar("SELECT count(*) FROM voip_tokens")
        .fetch_one(&database.pool)
        .await
        .expect("VoIP tokens");
    assert_eq!(
        voip, 0,
        "the removed device must no longer ring through PushKit"
    );
}

#[tokio::test]
async fn removing_a_device_removes_tokens_registered_before_the_device() {
    let database = start_database().await;
    let (app, _) = app(&database);
    let phone = account(&app).await;
    phone_tokens(&app, &phone).await;
    let device_id = device(&app, &phone).await;
    let laptop = second_session(&app).await;
    push(&app, &laptop, LAPTOP_PUSH).await;

    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/v1/devices/{device_id}"),
            &laptop,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_only_laptop_remains(&database).await;
}

#[tokio::test]
async fn pruning_an_idle_device_removes_even_recently_registered_tokens() {
    let database = start_database().await;
    let (app, ports) = app(&database);
    let phone = account(&app).await;
    let device_id = device(&app, &phone).await;
    phone_tokens(&app, &phone).await;
    let laptop = second_session(&app).await;
    push(&app, &laptop, LAPTOP_PUSH).await;

    sqlx::query("UPDATE devices SET last_seen_at = now() - interval '90 days' WHERE id = $1")
        .bind(device_id.parse::<i64>().expect("id"))
        .execute(&database.pool)
        .await
        .expect("age the device only");
    assert_eq!(
        application::use_cases::devices::prune_stale::execute(&ports)
            .await
            .expect("prune"),
        1
    );
    assert_only_laptop_remains(&database).await;
}

#[tokio::test]
async fn an_in_flight_registration_cannot_restore_tokens_after_logout() {
    let database = start_database().await;
    let (app, ports) = app(&database);
    let phone = account(&app).await;
    phone_tokens(&app, &phone).await;
    let user_id: i64 = sqlx::query_scalar("SELECT user_id FROM push_tokens")
        .fetch_one(&database.pool)
        .await
        .expect("user id");
    let users = [domain::Snowflake::new(user_id as u64)];
    // These are exactly the validated inputs an in-flight registration holds after
    // checking its session, before the repository starts its write transaction.
    let push = ports
        .push_tokens
        .list_live_for_users(&users)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let voip = ports
        .push_tokens
        .list_live_voip_for_users(&users)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(
        call(&app, Method::POST, "/api/v1/auth/logout", &phone, None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );

    assert!(
        matches!(
            ports.push_tokens.upsert(&push).await,
            Err(application::AppError::Unauthorized)
        ),
        "the write must recheck and lock the session, not trust an earlier authorization snapshot"
    );
    assert!(matches!(
        ports.push_tokens.upsert_voip(&voip).await,
        Err(application::AppError::Unauthorized)
    ));
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM push_tokens), (SELECT count(*) FROM voip_tokens)",
    )
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(counts, (0, 0));
}

#[tokio::test]
async fn registration_waits_for_an_uncommitted_revocation_and_then_refuses_both_token_kinds() {
    let database = start_database().await;
    let (app, _) = app(&database);
    let phone = account(&app).await;
    phone_tokens(&app, &phone).await;
    let session_id: uuid::Uuid = sqlx::query_scalar("SELECT session_id FROM push_tokens")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    // Pause a revocation after it locks/updates the session but before it deletes
    // the tokens and commits. Other connections still see the old live session.
    let mut revocation = database.pool.begin().await.unwrap();
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *revocation)
        .await
        .unwrap();
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = $1")
        .bind(session_id)
        .execute(&mut *revocation)
        .await
        .unwrap();
    let push_app = app.clone();
    let push_access = phone.clone();
    let push = tokio::spawn(async move {
        call(
            &push_app,
            Method::POST,
            "/api/v1/users/@me/devices/push",
            &push_access,
            Some(json!({"platform":"fcm", "token":"replacement-fcm-token"})),
        )
        .await
        .0
    });
    let voip = tokio::spawn(async move {
        call(
            &app,
            Method::POST,
            "/api/v1/users/@me/devices/voip",
            &phone,
            Some(json!({"token":PHONE_VOIP})),
        )
        .await
        .0
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)))",
            ).bind(blocker).fetch_one(&database.pool).await.unwrap();
            if waiting { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("registration must wait on the session being revoked");
    assert!(!push.is_finished() && !voip.is_finished());
    for table in ["push_tokens", "voip_tokens"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE session_id = $1"))
            .bind(session_id)
            .execute(&mut *revocation)
            .await
            .unwrap();
    }
    revocation.commit().await.unwrap();
    assert_eq!(push.await.unwrap(), StatusCode::UNAUTHORIZED);
    assert_eq!(voip.await.unwrap(), StatusCode::UNAUTHORIZED);
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM push_tokens), (SELECT count(*) FROM voip_tokens)",
    )
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(counts, (0, 0));
}

#[tokio::test]
async fn replacing_the_sessions_device_moves_its_push_association_and_keeps_the_new_tokens() {
    let database = start_database().await;
    let (app, ports) = app(&database);
    let phone = account(&app).await;
    phone_tokens(&app, &phone).await;
    let first_device = device(&app, &phone).await;
    let user_id: i64 = sqlx::query_scalar("SELECT user_id FROM push_tokens")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    let users = [domain::Snowflake::new(user_id as u64)];
    let old_input = ports
        .push_tokens
        .list_live_for_users(&users)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(
        old_input.device_id.map(|id| id.to_string()),
        Some(first_device.clone())
    );
    let replacement = device(&app, &phone).await;
    // A request validated before rebinding must use the session's current device
    // inside its write transaction, not restore the old device association.
    ports.push_tokens.upsert(&old_input).await.unwrap();
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/v1/devices/{first_device}"),
            &phone,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let current = ports.push_tokens.list_live_for_users(&users).await.unwrap();
    assert_eq!(current.len(), 1);
    assert_eq!(
        current[0].device_id.map(|id| id.to_string()),
        Some(replacement.clone())
    );
    assert_eq!(
        ports
            .push_tokens
            .list_live_voip_for_users(&users)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/v1/devices/{replacement}"),
            &phone,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert!(ports
        .push_tokens
        .list_live_for_users(&users)
        .await
        .unwrap()
        .is_empty());
    assert!(ports
        .push_tokens
        .list_live_voip_for_users(&users)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn failed_device_removal_rolls_back_token_cleanup_and_session_unbinding() {
    let database = start_database().await;
    let (app, _) = app(&database);
    let phone = account(&app).await;
    let device_id = device(&app, &phone).await;
    phone_tokens(&app, &phone).await;
    let events_before: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    sqlx::raw_sql(
        "CREATE FUNCTION fail_device_removal() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected device deletion failure'; END $$; \
         CREATE TRIGGER fail_device_removal BEFORE DELETE ON devices \
         FOR EACH ROW EXECUTE FUNCTION fail_device_removal();",
    )
    .execute(&database.pool)
    .await
    .unwrap();
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/v1/devices/{device_id}"),
            &phone,
            None
        )
        .await
        .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let remaining: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM push_tokens), (SELECT count(*) FROM voip_tokens), \
         (SELECT count(*) FROM sessions WHERE device_id = $1), (SELECT count(*) FROM devices WHERE id = $1)",
    ).bind(device_id.parse::<i64>().unwrap()).fetch_one(&database.pool).await.unwrap();
    assert_eq!(
        remaining,
        (1, 1, 1, 1),
        "token cleanup, unbinding and device deletion are one transaction"
    );
    let events_after: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    assert_eq!(
        events_after, events_before,
        "failed cleanup must not announce a removed device"
    );
}
