//! HTTP contract tests for applications, bot commands, OAuth2 bot
//! authorization and webhooks.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
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
async fn applications_commands_authorization_and_webhooks_follow_the_contract() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let owner = register(&app, "dev@example.com", "developer").await;

    let (status, created) = call(
        &app,
        Method::POST,
        "/api/v1/applications",
        &owner,
        Some(json!({"name": "Helper Bot", "description": "Helps"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let application_id = created["id"].as_str().expect("application id").to_owned();
    let bot_id = created["bot"]["id"]
        .as_str()
        .expect("bot user id")
        .to_owned();
    assert_eq!(created["bot"]["bot"], true);

    let (status, command) = call(
        &app,
        Method::POST,
        &format!("/api/v1/applications/{application_id}/commands"),
        &owner,
        Some(json!({"name": "ping", "description": "Replies pong", "type": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let command_id = command["id"].as_str().expect("command id").to_owned();

    let (status, listed) = call(
        &app,
        Method::GET,
        &format!("/api/v1/applications/{application_id}/commands"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed.as_array().map(Vec::len), Some(1));

    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/v1/applications/{application_id}/commands/{command_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name": "Bots"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild_id = guild["id"].as_str().expect("guild id").to_owned();

    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/oauth2/authorize",
        &owner,
        Some(json!({
            "client_id": application_id,
            "guild_id": guild_id,
            "permissions": "8",
            "installation_revision": "absent:0",
            "scope": "bot applications.commands"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the owner can install the bot");

    let (status, installed) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/members?limit=1000"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let bot_member = installed
        .as_array()
        .and_then(|members| members.iter().find(|m| m["user"]["id"] == bot_id.as_str()))
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(bot_member["user"]["bot"], true, "the bot joins the guild");

    let (status, channels) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let channel_id = channels
        .as_array()
        .and_then(|list| list.iter().find(|c| c["type"] == 0))
        .and_then(|c| c["id"].as_str())
        .expect("a text channel")
        .to_owned();

    let (status, hook) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{channel_id}/webhooks"),
        &owner,
        Some(json!({"name": "deploys"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{hook}");
    let webhook_id = hook["id"].as_str().expect("webhook id").to_owned();
    let token = hook["token"]
        .as_str()
        .expect("token is shown on creation")
        .to_owned();
    assert!(hook["url"]
        .as_str()
        .unwrap_or_default()
        .contains(&format!("/api/v1/webhooks/{webhook_id}/{token}")));

    let (status, listed_hooks) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/webhooks"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed_hooks.as_array().map(Vec::len), Some(1));
    assert!(
        listed_hooks[0].get("token").is_none(),
        "tokens are only returned on creation"
    );

    let (status, _) = send(
        &app,
        request(
            Method::POST,
            &format!("/api/v1/webhooks/{webhook_id}/not-the-token"),
            None,
            Some(json!({"content": "hi"})),
        ),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "a wrong webhook token must not execute"
    );

    let (status, renamed) = call(
        &app,
        Method::PATCH,
        &format!("/api/v1/webhooks/{webhook_id}"),
        &owner,
        Some(json!({"name": "releases"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(renamed["name"], "releases");

    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/v1/webhooks/{webhook_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}
