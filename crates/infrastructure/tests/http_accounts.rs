//! HTTP contract tests for accounts, devices and the internal API, driven
//! through the real router against a testcontainers Postgres.

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use interface::state::AppState;
use support::{jwt_keys, ports, start_database};

const INTERNAL_TOKEN: &str = "test-internal-token";

async fn send(app: &axum::Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.expect("router responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body)
}

fn post(path: &str, token: Option<&str>, body: Value) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(Body::from(body.to_string())).expect("request")
}

fn get(path: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(path);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(Body::empty()).expect("request")
}

fn patch(path: &str, token: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("PATCH")
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .expect("request")
}

#[tokio::test]
async fn accounts_devices_and_internal_endpoints_follow_the_contract() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let state = AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    };
    let app = interface::router(state);

    let (status, alice) = send(
        &app,
        post(
            "/api/v1/auth/register",
            None,
            json!({"email": "alice@example.com", "username": "alice", "password": "a-long-enough-passphrase"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let alice_token = alice["access_token"].as_str().expect("token").to_owned();
    let alice_id = alice["user"]["id"].as_str().expect("id").to_owned();

    let (status, bob) = send(
        &app,
        post(
            "/api/v1/auth/register",
            None,
            json!({"email": "bob@example.com", "username": "bob", "password": "a-long-enough-passphrase"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let bob_id = bob["user"]["id"].as_str().expect("id").to_owned();

    let (status, body) = send(&app, get("/api/v1/users/@me", None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], 40001);

    let (status, me) = send(&app, get("/api/v1/users/@me", Some(&alice_token))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["username"], "alice");
    assert_eq!(
        me["email"], "alice@example.com",
        "self view includes the email"
    );

    let (status, updated) = send(
        &app,
        patch(
            "/api/v1/users/@me",
            &alice_token,
            json!({"global_name": "Alice A."}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["global_name"], "Alice A.");

    let (status, public_view) = send(
        &app,
        get(&format!("/api/v1/users/{bob_id}"), Some(&alice_token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        public_view.get("email").is_none(),
        "other users see no email"
    );

    let (status, body) = send(
        &app,
        post(
            "/api/v1/auth/login",
            None,
            json!({"email": "alice@example.com", "password": "wrong-password-here"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], 50035);

    let (status, device) = send(
        &app,
        post(
            "/api/v1/devices",
            Some(&alice_token),
            json!({"name": "laptop", "identity_key_b64": "AAAA"}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "identity key must be exactly 32 bytes"
    );
    assert_eq!(device["code"], 50035);

    let identity_key = base64_of_32_bytes();
    let (status, device) = send(
        &app,
        post(
            "/api/v1/devices",
            Some(&alice_token),
            json!({"name": "laptop", "identity_key_b64": identity_key}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(device["device_id"].is_string());
    let device_id = device["device_id"].as_str().expect("device id").to_owned();

    let (status, devices) = send(
        &app,
        get(
            &format!("/api/v1/users/{alice_id}/devices"),
            Some(&alice_token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(devices.as_array().map(Vec::len), Some(1));

    let (status, dm) = send(
        &app,
        post(
            "/api/v1/users/@me/channels",
            Some(&alice_token),
            json!({"recipient_ids": [bob_id]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dm["type"], 1);

    let (status, introspected) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri("/internal/v1/auth/introspect")
            .header("content-type", "application/json")
            .body(Body::from(json!({"token": alice_token}).to_string()))
            .expect("request"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "internal calls need the shared token"
    );
    assert_eq!(introspected["code"], 40001);

    let (status, introspected) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri("/internal/v1/auth/introspect")
            .header("content-type", "application/json")
            .header("x-internal-token", INTERNAL_TOKEN)
            .body(Body::from(json!({"token": alice_token}).to_string()))
            .expect("request"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(introspected["kind"], "user");
    assert_eq!(introspected["user_id"], alice_id);

    let (status, ready) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!("/internal/v1/users/{alice_id}/ready"))
            .header("x-internal-token", INTERNAL_TOKEN)
            .body(Body::empty())
            .expect("request"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ready["user"]["id"], alice_id);
    assert_eq!(ready["private_channels"].as_array().map(Vec::len), Some(1));

    let (status, _) = send(
        &app,
        get(&format!("/api/v1/devices/{device_id}"), Some(&alice_token)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::METHOD_NOT_ALLOWED,
        "the device path only accepts DELETE"
    );
}

fn base64_of_32_bytes() -> String {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    STANDARD.encode([7_u8; 32])
}
