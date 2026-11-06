//! HTTP contract tests for PushKit (VoIP) tokens: registration per session, independence from the
//! notification token, removal at logout and by the notifier, and the internal push-targets
//! endpoint that carries them.

mod support;

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database, Database};

const PASSWORD: &str = "a-long-enough-passphrase";
const VOIP_A: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const VOIP_B: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const PUSH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn app(database: &Database) -> axum::Router {
    let (private, public) = jwt_keys();
    interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    })
}

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

async fn put_voip(app: &axum::Router, access: &str, token: &str) -> StatusCode {
    call(
        app,
        Method::POST,
        "/api/v1/users/@me/devices/voip",
        access,
        Some(json!({"token": token, "environment": "sandbox"})),
    )
    .await
    .0
}

async fn stored(database: &Database) -> Vec<(i64, String, String)> {
    sqlx::query_as("SELECT user_id, token, environment FROM voip_tokens ORDER BY created_at, token")
        .fetch_all(&database.pool)
        .await
        .expect("voip_tokens query")
}

fn internal(
    method: Method,
    path: &str,
    body: Option<Value>,
) -> axum::http::Request<axum::body::Body> {
    let builder = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("x-internal-token", INTERNAL_TOKEN);
    match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .expect("request"),
        None => builder.body(axum::body::Body::empty()).expect("request"),
    }
}

async fn targets(app: &axum::Router, channel_id: &str, author_id: &str) -> Value {
    let (status, body) = send(
        app,
        internal(
            Method::GET,
            &format!("/internal/v1/channels/{channel_id}/push-targets?author_id={author_id}"),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "push-targets: {body}");
    body
}

async fn dm(app: &axum::Router, from: &str, to_id: &str) -> String {
    let (status, channel) = call(
        app,
        Method::POST,
        "/api/v1/users/@me/channels",
        from,
        Some(json!({"recipient_ids": [to_id]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    channel["id"].as_str().expect("channel id").to_owned()
}

#[tokio::test]
async fn a_session_keeps_one_voip_token_that_a_refresh_replaces_and_a_delete_removes() {
    let database = start_database().await;
    let app = app(&database);
    let (alice, alice_id) = register(&app, "alice").await;

    assert_eq!(put_voip(&app, &alice, VOIP_A).await, StatusCode::NO_CONTENT);
    assert_eq!(
        stored(&database).await,
        vec![(
            alice_id.parse::<i64>().expect("id"),
            VOIP_A.to_owned(),
            "sandbox".to_owned()
        )]
    );

    assert_eq!(put_voip(&app, &alice, VOIP_B).await, StatusCode::NO_CONTENT);
    let rows = stored(&database).await;
    assert_eq!(rows.len(), 1, "a session holds one VoIP token");
    assert_eq!(rows[0].1, VOIP_B);

    let (status, _) = call(
        &app,
        Method::DELETE,
        "/api/v1/users/@me/devices/voip",
        &alice,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(stored(&database).await.is_empty());
    let (status, _) = call(
        &app,
        Method::DELETE,
        "/api/v1/users/@me/devices/voip",
        &alice,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "removing nothing is not an error"
    );
}

#[tokio::test]
async fn registration_validates_the_request_and_needs_a_user_session() {
    let database = start_database().await;
    let app = app(&database);
    let (alice, _) = register(&app, "alice").await;

    for body in [
        json!({"token": "not hex!"}),
        json!({"token": ""}),
        json!({"token": VOIP_A, "environment": "staging"}),
    ] {
        let (status, _) = call(
            &app,
            Method::POST,
            "/api/v1/users/@me/devices/voip",
            &alice,
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
    let (status, _) = send(
        &app,
        request(
            Method::POST,
            "/api/v1/users/@me/devices/voip",
            None,
            Some(json!({"token": VOIP_A})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(stored(&database).await.is_empty());
}

#[tokio::test]
async fn the_same_installation_moving_to_another_account_takes_its_voip_token_along() {
    let database = start_database().await;
    let app = app(&database);
    let (alice, _) = register(&app, "alice").await;
    let (bob, bob_id) = register(&app, "bob").await;

    assert_eq!(put_voip(&app, &alice, VOIP_A).await, StatusCode::NO_CONTENT);
    assert_eq!(put_voip(&app, &bob, VOIP_A).await, StatusCode::NO_CONTENT);

    let rows = stored(&database).await;
    assert_eq!(rows.len(), 1, "a PushKit token belongs to one session");
    assert_eq!(rows[0].0, bob_id.parse::<i64>().expect("id"));
}

#[tokio::test]
async fn logout_removes_the_voip_token_of_that_session() {
    let database = start_database().await;
    let app = app(&database);
    let (phone, _) = register(&app, "alice").await;
    assert_eq!(put_voip(&app, &phone, VOIP_A).await, StatusCode::NO_CONTENT);

    let (status, _) = call(&app, Method::POST, "/api/v1/auth/logout", &phone, None).await;
    assert!(status.is_success(), "logout: {status}");
    assert!(stored(&database).await.is_empty());
}

#[tokio::test]
async fn the_notifier_removes_a_rejected_voip_token_through_the_internal_api() {
    let database = start_database().await;
    let app = app(&database);
    let (alice, _) = register(&app, "alice").await;
    assert_eq!(put_voip(&app, &alice, VOIP_A).await, StatusCode::NO_CONTENT);

    let (status, _) = send(
        &app,
        internal(
            Method::DELETE,
            "/internal/v1/push-tokens",
            Some(json!({"platform": "apns", "token": VOIP_A})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(stored(&database).await.is_empty());
}

#[tokio::test]
async fn push_targets_list_voip_tokens_even_for_a_user_without_a_notification_token() {
    let database = start_database().await;
    let app = app(&database);
    let (alice, alice_id) = register(&app, "alice").await;
    let (bob, bob_id) = register(&app, "bob").await;
    let channel = dm(&app, &alice, &bob_id).await;

    // Bob denied notifications: he only has a PushKit token. It is the only way his phone rings.
    assert_eq!(put_voip(&app, &bob, VOIP_B).await, StatusCode::NO_CONTENT);
    assert_eq!(put_voip(&app, &alice, VOIP_A).await, StatusCode::NO_CONTENT);

    let found = targets(&app, &channel, &alice_id).await;
    let list = found["targets"].as_array().expect("targets");
    assert_eq!(list.len(), 1, "the caller is not a target");
    assert_eq!(list[0]["user_id"], bob_id.as_str());
    assert_eq!(list[0]["tokens"], json!([]));
    assert_eq!(
        list[0]["voip_tokens"],
        json!([{"token": VOIP_B, "environment": "sandbox"}])
    );

    // With a notification token too, both lists are filled.
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/users/@me/devices/push",
        &bob,
        Some(json!({"platform": "apns", "token": PUSH_B, "environment": "sandbox"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let list = targets(&app, &channel, &alice_id).await["targets"].clone();
    assert_eq!(list.as_array().expect("targets").len(), 1);
    assert_eq!(list[0]["tokens"].as_array().expect("tokens").len(), 1);
    assert_eq!(list[0]["voip_tokens"].as_array().expect("voip").len(), 1);
}
