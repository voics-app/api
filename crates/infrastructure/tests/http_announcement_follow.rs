//! The public follow route persists a non-executable webhook in the target guild.
mod support;

use axum::http::{Method, StatusCode};
use serde_json::json;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database};

#[tokio::test]
async fn follow_requires_visible_announcement_and_target_webhook_permission() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(interface::state::AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let (status, user) = send(&app, request(Method::POST, "/api/v1/auth/register", None,
        Some(json!({"email":"follow@example.com", "username":"follower", "password":"a-long-enough-passphrase"})))).await;
    assert_eq!(status, StatusCode::CREATED);
    let token = user["access_token"].as_str().unwrap();
    let (_, source_guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        token,
        Some(json!({"name":"Source"})),
    )
    .await;
    let (_, target_guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        token,
        Some(json!({"name":"Target"})),
    )
    .await;
    let source_guild_id = source_guild["id"].as_str().unwrap();
    let target_guild_id = target_guild["id"].as_str().unwrap();
    let (status, source) = call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{source_guild_id}/channels"),
        token,
        Some(json!({"name":"news", "type":5})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{source}");
    let source_id = source["id"].as_str().unwrap();
    let (_, channels) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{target_guild_id}/channels"),
        token,
        None,
    )
    .await;
    let target_id = channels
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == 0)
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let uri = format!("/api/v1/channels/{source_id}/followers");
    let (status, follower) = call(
        &app,
        Method::POST,
        &uri,
        token,
        Some(json!({"webhook_channel_id":target_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{follower}");
    assert_eq!(follower["channel_id"], source_id);
    let hook_id = follower["webhook_id"].as_str().unwrap();
    let (status, hook) = call(
        &app,
        Method::GET,
        &format!("/api/v1/webhooks/{hook_id}"),
        token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(hook["type"], 2);
    assert!(hook.get("token").is_none());
    let (status, following) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{target_guild_id}/followed-channels"),
        token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(following[0]["channel_id"], source_id);
    assert_eq!(following[0]["webhook_channel_id"], target_id);
    assert_eq!(following[0]["webhook_id"], hook_id);
    let (status, outsider) = send(
        &app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({"email":"outsider@example.com", "username":"outsider", "password":"a-long-enough-passphrase"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let outsider_token = outsider["access_token"].as_str().unwrap();
    let (status, _) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{target_guild_id}/followed-channels"),
        outsider_token,
        None,
    )
    .await;
    assert!(status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND);
    let (status, _) = call(
        &app,
        Method::POST,
        &uri,
        outsider_token,
        Some(json!({"webhook_channel_id":target_id})),
    )
    .await;
    assert!(status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND);
    let (status, _) = call(
        &app,
        Method::POST,
        &uri,
        token,
        Some(json!({"webhook_channel_id":target_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{target_id}/followers"),
        token,
        Some(json!({"webhook_channel_id":source_id})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/v1/channels/{source_id}"),
        token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        &app,
        Method::GET,
        &format!("/api/v1/webhooks/{hook_id}"),
        token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
