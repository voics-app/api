//! HTTP contract tests for friend requests, driven through the real router
//! against a testcontainers Postgres.

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

fn with_token(builder: axum::http::request::Builder, token: &str) -> axum::http::request::Builder {
    builder.header("authorization", format!("Bearer {token}"))
}

fn post_json(path: &str, token: &str, body: Value) -> Request<Body> {
    with_token(
        Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json"),
        token,
    )
    .body(Body::from(body.to_string()))
    .expect("request")
}

fn get(path: &str, token: &str) -> Request<Body> {
    with_token(Request::builder().method("GET").uri(path), token)
        .body(Body::empty())
        .expect("request")
}

fn assert_public_user(user: &Value, username: &str) {
    assert_eq!(user["username"], username);
    for key in ["id", "global_name", "avatar"] {
        assert!(user.get(key).is_some(), "public user carries `{key}`");
    }
    assert!(user.get("email").is_none(), "public user has no email");
}

#[tokio::test]
async fn adding_a_friend_by_username_lists_the_full_public_user_for_both_sides() {
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
        post_json(
            "/api/v1/auth/register",
            "",
            json!({"email": "alice@example.com", "username": "alice", "password": "a-long-enough-passphrase"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let alice_token = alice["access_token"].as_str().expect("token").to_owned();

    let (status, bob) = send(
        &app,
        post_json(
            "/api/v1/auth/register",
            "",
            json!({"email": "bob@example.com", "username": "bob", "password": "a-long-enough-passphrase"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let bob_token = bob["access_token"].as_str().expect("token").to_owned();

    let (status, _) = send(
        &app,
        post_json(
            "/api/v1/users/@me/relationships",
            &alice_token,
            json!({"username": "bob"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, alice_list) =
        send(&app, get("/api/v1/users/@me/relationships", &alice_token)).await;
    assert_eq!(status, StatusCode::OK);
    let alice_rows = alice_list.as_array().expect("array");
    assert_eq!(alice_rows.len(), 1);
    assert_public_user(&alice_rows[0]["user"], "bob");

    let (status, bob_list) = send(&app, get("/api/v1/users/@me/relationships", &bob_token)).await;
    assert_eq!(status, StatusCode::OK);
    let bob_rows = bob_list.as_array().expect("array");
    assert_eq!(bob_rows.len(), 1);
    assert_public_user(&bob_rows[0]["user"], "alice");

    let (status, body) = send(
        &app,
        post_json(
            "/api/v1/users/@me/relationships",
            &alice_token,
            json!({"username": "nobody-here"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], 10013);
}
