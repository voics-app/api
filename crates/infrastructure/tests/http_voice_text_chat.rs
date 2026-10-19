//! HTTP contract tests for the text chat that every voice channel carries:
//! on by default, switchable by channel managers, and only for voice channels.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send};
use support::{jwt_keys, ports, start_database};

async fn register(app: &axum::Router, email: &str, username: &str) -> String {
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
    body["access_token"].as_str().expect("token").to_owned()
}

#[tokio::test]
async fn voice_text_chat_is_on_by_default_and_toggles_for_voice_only() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let owner = register(&app, "owner@example.com", "owner").await;

    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name": "Voice HQ"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild_id = guild["id"].as_str().expect("guild id").to_owned();

    let (status, voice) = call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        Some(json!({"name": "lounge", "type": 2})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let voice_id = voice["id"].as_str().expect("voice id").to_owned();
    assert_eq!(
        voice["text_chat_enabled"], true,
        "text chat is on by default"
    );

    let (status, text) = call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        Some(json!({"name": "general-talk", "type": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let text_id = text["id"].as_str().expect("text id").to_owned();

    let (status, updated) = call(
        &app,
        Method::PATCH,
        &format!("/api/v1/channels/{voice_id}"),
        &owner,
        Some(json!({"text_chat_enabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["text_chat_enabled"], false);

    let (status, fetched): (StatusCode, Value) = call(
        &app,
        Method::GET,
        &format!("/api/v1/channels/{voice_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["text_chat_enabled"], false, "the setting persists");

    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/api/v1/channels/{text_id}"),
        &owner,
        Some(json!({"text_chat_enabled": false})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "only voice channels have a text chat"
    );
}
