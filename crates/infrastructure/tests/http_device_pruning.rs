//! Device activity (api#25): stale devices leave the E2EE listings and key-package claims, a
//! token refresh keeps a device alive, and the prune removes what stayed idle.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use application::use_cases::devices::prune_stale;
use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database};

const KEY_B64: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const PASSWORD: &str = "a-long-enough-passphrase";

async fn register(app: &axum::Router, username: &str) -> (String, String) {
    let (status, body) = send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({
                "email": format!("{username}@example.com"),
                "username": username,
                "password": PASSWORD,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "register {username}");
    (
        body["access_token"].as_str().expect("token").to_owned(),
        body["user"]["id"].as_str().expect("id").to_owned(),
    )
}

/// A fresh session of `username` with its own registered device: (access token, refresh token, device id).
async fn sign_in_with_device(app: &axum::Router, username: &str) -> (String, String, String) {
    let (status, session) = send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/login",
            None,
            Some(json!({"email": format!("{username}@example.com"), "password": PASSWORD})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "login {username}");
    let access = session["access_token"].as_str().expect("access").to_owned();
    let refresh = session["refresh_token"]
        .as_str()
        .expect("refresh")
        .to_owned();
    let (status, device) = call(
        app,
        Method::POST,
        "/api/v1/devices",
        &access,
        Some(json!({"name": username, "identity_key_b64": KEY_B64})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "register device");
    (
        device["access_token"].as_str().expect("bound").to_owned(),
        refresh,
        device["device_id"].as_str().expect("device id").to_owned(),
    )
}

async fn listed(app: &axum::Router, viewer: &str, user_id: &str) -> Vec<String> {
    let (status, devices) = call(
        app,
        Method::GET,
        &format!("/api/v1/users/{user_id}/devices"),
        viewer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    devices
        .as_array()
        .expect("device list")
        .iter()
        .map(|d| d["device_id"].as_str().expect("id").to_owned())
        .collect()
}

async fn age(database: &support::Database, device_id: &str, days: i32) {
    sqlx::query(
        "UPDATE devices SET last_seen_at = now() - make_interval(days => $2) WHERE id = $1",
    )
    .bind(device_id.parse::<i64>().expect("snowflake"))
    .bind(days)
    .execute(&database.pool)
    .await
    .expect("age device");
}

#[tokio::test]
async fn stale_devices_are_hidden_kept_alive_by_refresh_and_pruned() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let ports = ports(&database.pool, &public, &private);
    let app = interface::router(AppState {
        ports: ports.clone(),
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });

    let (alice, alice_id) = register(&app, "alice").await;
    let (bob, _bob_id) = register(&app, "bob").await;
    let (alice_a, _refresh_a, device_a) = sign_in_with_device(&app, "alice").await;
    let (alice_b, refresh_b, device_b) = sign_in_with_device(&app, "alice").await;
    let _ = alice;

    for (token, device) in [(&alice_a, &device_a), (&alice_b, &device_b)] {
        let (status, _) = call(
            &app,
            Method::POST,
            &format!("/api/v1/devices/{device}/key-packages"),
            token,
            Some(json!({"packages": ["cGFja2FnZQ=="]})),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "upload key package");
    }
    assert_eq!(listed(&app, &bob, &alice_id).await.len(), 2);

    // Idle for longer than the default 30 days: hidden, and nobody can claim a key package.
    age(&database, &device_a, 40).await;
    assert_eq!(listed(&app, &bob, &alice_id).await, vec![device_b.clone()]);
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/v1/users/{alice_id}/devices/{device_a}/key-packages/claim"),
        &bob,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "stale device is not claimable"
    );
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/v1/users/{alice_id}/devices/{device_b}/key-packages/claim"),
        &bob,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "active device is claimable");

    // A token refresh of the device's session marks it seen again.
    age(&database, &device_b, 10).await;
    let (status, _) = send(
        &app,
        request(
            Method::POST,
            "/api/v1/auth/refresh",
            None,
            Some(json!({"refresh_token": refresh_b})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "refresh");
    let (fresh,): (bool,) = sqlx::query_as(
        "SELECT last_seen_at > now() - interval '1 minute' FROM devices WHERE id = $1",
    )
    .bind(device_b.parse::<i64>().expect("snowflake"))
    .fetch_one(&database.pool)
    .await
    .expect("last seen");
    assert!(fresh, "refresh bumps last_seen_at");

    // The prune deletes the stale device (with its key packages), keeps the rest and announces the
    // remaining list.
    let before = device_count(&database).await;
    assert_eq!(prune_stale::execute(&ports).await.expect("prune"), 1);
    assert_eq!(device_count(&database).await, before - 1);
    assert_eq!(listed(&app, &bob, &alice_id).await, vec![device_b.clone()]);
    let (orphans,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM key_packages WHERE device_id = $1")
            .bind(device_a.parse::<i64>().expect("snowflake"))
            .fetch_one(&database.pool)
            .await
            .expect("count");
    assert_eq!(orphans, 0, "key packages go with the device");
    let (last,): (Value,) = sqlx::query_as(
        "SELECT payload FROM outbox WHERE payload->>'type' = 'USER_DEVICES_UPDATE' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&database.pool)
    .await
    .expect("update");
    assert_eq!(last["data"]["user_id"], alice_id);
    assert_eq!(
        last["data"]["devices"].as_array().map(Vec::len),
        Some(1),
        "the update carries only the remaining device"
    );
    assert_eq!(
        prune_stale::execute(&ports).await.expect("second prune"),
        0,
        "nothing left to prune"
    );
}

async fn device_count(database: &support::Database) -> i64 {
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM devices")
        .fetch_one(&database.pool)
        .await
        .expect("count");
    count
}
