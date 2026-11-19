//! Screening through public HTTP and the internal message authorization boundary.
mod support;

use axum::http::{Method, StatusCode};
use serde_json::json;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database};

async fn register(app: &axum::Router, name: &str) -> (String, String) {
    let (status, body) = send(app, request(Method::POST, "/api/v1/auth/register", None,
        Some(json!({"email": format!("{name}@example.com"), "username": name, "password": "a-long-enough-passphrase"})))).await;
    assert_eq!(status, StatusCode::CREATED);
    (
        body["access_token"].as_str().unwrap().into(),
        body["user"]["id"].as_str().unwrap().into(),
    )
}

async fn permissions(app: &axum::Router, channel: &str, user: &str) -> u64 {
    let req = axum::http::Request::builder()
        .uri(format!("/internal/v1/channels/{channel}/access/{user}"))
        .header("x-internal-token", INTERNAL_TOKEN)
        .body(axum::body::Body::empty())
        .unwrap();
    let (status, body) = send(app, req).await;
    assert_eq!(status, StatusCode::OK);
    body["permissions"].as_str().unwrap().parse().unwrap()
}

#[tokio::test]
async fn join_and_accept_current_rules_before_sending() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(interface::state::AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: Default::default(),
    });
    let (owner, _) = register(&app, "screen_owner").await;
    let (guest, guest_id) = register(&app, "screen_guest").await;
    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name":"Rules"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild_id = guild["id"].as_str().unwrap();
    let path = format!("/api/v1/guilds/{guild_id}/member-verification");
    let (status, first) = call(
        &app,
        Method::PUT,
        &path,
        &owner,
        Some(json!({"enabled":true,"rules":["Be kind"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, channels) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        None,
    )
    .await;
    let channel = channels
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == 0)
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let (_, invite) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{channel}/invites"),
        &owner,
        Some(json!({})),
    )
    .await;
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/v1/invites/{}", invite["code"].as_str().unwrap()),
        &guest,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(permissions(&app, channel, &guest_id).await & (1 << 11), 0);
    let (_, shown) = call(&app, Method::GET, &path, &guest, None).await;
    assert_eq!(shown["pending"], true);
    assert_eq!(shown["revision"], first["revision"]);
    let (_, changed) = call(
        &app,
        Method::PUT,
        &path,
        &owner,
        Some(json!({"enabled":true,"rules":["Be kind", "No spam"]})),
    )
    .await;
    let accept = format!("{path}/accept");
    let (status, _) = call(
        &app,
        Method::POST,
        &accept,
        &guest,
        Some(json!({"revision": first["revision"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(permissions(&app, channel, &guest_id).await & (1 << 11), 0);
    let (status, _) = call(
        &app,
        Method::POST,
        &accept,
        &guest,
        Some(json!({"revision": changed["revision"]})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_ne!(permissions(&app, channel, &guest_id).await & (1 << 11), 0);
}
