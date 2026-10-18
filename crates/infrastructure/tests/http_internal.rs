//! HTTP contract tests for the internal P4 endpoints owned by api: guild
//! state, channel access, admin statistics, user search and suspension.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::json;

use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
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
    assert_eq!(status, StatusCode::CREATED);
    (
        body["access_token"].as_str().expect("token").to_owned(),
        body["user"]["id"].as_str().expect("id").to_owned(),
    )
}

fn internal(method: Method, path: &str) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("x-internal-token", INTERNAL_TOKEN)
        .body(axum::body::Body::empty())
        .expect("request")
}

#[tokio::test]
async fn internal_endpoints_require_the_token_and_serve_state() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: INTERNAL_TOKEN.into(),
    });
    let (alice_token, alice_id) = register(&app, "alice@example.com", "alice").await;
    let (_bob_token, bob_id) = register(&app, "bob@example.com", "bob").await;

    let (status, _) = send(
        &app,
        request(Method::GET, "/internal/v1/admin/stats", None, None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &alice_token,
        Some(json!({"name": "Internal"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild_id = guild["id"].as_str().expect("guild id").to_owned();

    let (status, state) = send(
        &app,
        internal(
            Method::GET,
            &format!("/internal/v1/guilds/{guild_id}/state"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state["name"], "Internal");
    assert!(state["channels"]
        .as_array()
        .map(|c| !c.is_empty())
        .unwrap_or(false));

    let channel_id = state["channels"]
        .as_array()
        .and_then(|list| list.iter().find(|c| c["type"] == 0))
        .and_then(|c| c["id"].as_str())
        .expect("a text channel")
        .to_owned();
    let (status, access) = send(
        &app,
        internal(
            Method::GET,
            &format!("/internal/v1/channels/{channel_id}/access/{alice_id}"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(access["permissions"].is_string());
    assert_eq!(access["guild_id"], guild_id);

    let (status, _) = send(
        &app,
        internal(
            Method::GET,
            &format!("/internal/v1/channels/{channel_id}/access/{bob_id}"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "non-members get 403");

    let (status, stats) = send(&app, internal(Method::GET, "/internal/v1/admin/stats")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stats["users_total"], 2);
    assert_eq!(stats["guilds_total"], 1);

    let (status, page) = send(
        &app,
        internal(Method::GET, "/internal/v1/admin/users?query=bo&limit=10"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["users"][0]["username"], "bob");

    let (status, guilds) = send(
        &app,
        internal(Method::GET, "/internal/v1/admin/guilds?query=Internal"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(guilds[0]["member_count"], 1);

    let suspend = axum::http::Request::builder()
        .method(Method::POST)
        .uri(format!("/internal/v1/admin/users/{bob_id}/suspend"))
        .header("x-internal-token", INTERNAL_TOKEN)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            json!({"reason": "spam"}).to_string(),
        ))
        .expect("request");
    let (status, _) = send(&app, suspend).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = send(
        &app,
        request(
            Method::POST,
            "/api/v1/auth/login",
            None,
            Some(json!({"email": "bob@example.com", "password": "a-long-enough-passphrase"})),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "suspended accounts cannot sign in"
    );
}
