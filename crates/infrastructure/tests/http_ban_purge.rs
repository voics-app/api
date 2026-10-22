//! HTTP contract tests for `PUT /guilds/{id}/bans/{uid}` with `delete_message_seconds`:
//! the ban writes a BAN_MESSAGE_PURGE bus event listing the guild's channels and the window start.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send};
use support::{jwt_keys, ports, start_database};

async fn register(app: &axum::Router, email: &str, username: &str) -> (String, String) {
    let (status, body) = send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({"email": email, "username": username, "password": "a-long-enough-passphrase"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "register {username}");
    (
        body["access_token"].as_str().expect("token").to_owned(),
        body["user"]["id"].as_str().expect("id").to_owned(),
    )
}

async fn create_channel(app: &axum::Router, token: &str, guild_id: &str, kind: u8) -> String {
    let (status, channel) = call(
        app,
        Method::POST,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        token,
        Some(json!({"name": "room", "type": kind})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    channel["id"].as_str().expect("channel id").to_owned()
}

async fn purge_events(pool: &sqlx::PgPool) -> Vec<Value> {
    let rows: Vec<(Value,)> = sqlx::query_as(
        "SELECT payload FROM outbox WHERE payload->>'type' = 'BAN_MESSAGE_PURGE' ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .expect("outbox query");
    rows.into_iter().map(|(payload,)| payload).collect()
}

#[tokio::test]
async fn ban_with_delete_message_seconds_emits_a_purge_event() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let (owner, _) = register(&app, "owner@example.com", "owner").await;
    let (_, guest_id) = register(&app, "guest@example.com", "guest").await;
    let (_, other_id) = register(&app, "other@example.com", "other").await;
    let (_, third_id) = register(&app, "third@example.com", "third").await;

    let (_, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name": "HQ"})),
    )
    .await;
    let guild_id = guild["id"].as_str().expect("guild id").to_owned();
    let text = create_channel(&app, &owner, &guild_id, 0).await;
    let voice = create_channel(&app, &owner, &guild_id, 2).await;
    let category = create_channel(&app, &owner, &guild_id, 4).await;

    let ban = |user: &str| format!("/api/v1/guilds/{guild_id}/bans/{user}");

    // No body, or zero seconds: a plain ban with no purge.
    let (status, _) = call(&app, Method::PUT, &ban(&other_id), &owner, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        &app,
        Method::PUT,
        &ban(&third_id),
        &owner,
        Some(json!({"delete_message_seconds": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(purge_events(&database.pool).await.is_empty());

    // Out of range is rejected before anything is written.
    let (status, _) = call(
        &app,
        Method::PUT,
        &ban(&guest_id),
        &owner,
        Some(json!({"delete_message_seconds": 604_801})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(purge_events(&database.pool).await.is_empty());

    let before = chrono::Utc::now().timestamp_millis();
    let (status, _) = call(
        &app,
        Method::PUT,
        &ban(&guest_id),
        &owner,
        Some(json!({"delete_message_seconds": 3600})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let events = purge_events(&database.pool).await;
    assert_eq!(events.len(), 1);
    let data = &events[0]["data"];
    assert_eq!(data["guild_id"], guild_id);
    assert_eq!(data["user_id"], guest_id);
    assert_eq!(data["delete_message_seconds"], 3600);
    let channels: Vec<&str> = data["channel_ids"]
        .as_array()
        .expect("channel ids")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(channels.contains(&text.as_str()) && channels.contains(&voice.as_str()));
    assert!(
        !channels.contains(&category.as_str()),
        "categories hold no messages"
    );
    let since = data["since_ms"].as_i64().expect("since_ms");
    assert!(
        since >= before - 3_600_000
            && since <= chrono::Utc::now().timestamp_millis() - 3_600_000 + 1_000
    );
}
