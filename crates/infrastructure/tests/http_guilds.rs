//! HTTP contract tests for guilds: creation, channels, roles, members, bans,
//! invites, emojis and the audit log.

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

#[tokio::test]
async fn guild_administration_follows_the_contract() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let (owner, owner_id) = register(&app, "owner@example.com", "owner").await;
    let (guest, guest_id) = register(&app, "guest@example.com", "guest").await;

    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name": "Voics HQ"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild_id = guild["id"].as_str().expect("guild id").to_owned();
    let everyone_id = guild_id.clone();
    assert_eq!(guild["name"], "Voics HQ");

    let (status, channels) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let general = channels
        .as_array()
        .and_then(|list| list.first())
        .cloned()
        .unwrap_or(Value::Null);
    assert!(general.get("id").is_some(), "the default channel exists");

    let (status, created) = call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        Some(json!({"name": "announcements", "type": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let channel_id = created["id"].as_str().expect("channel id").to_owned();

    let (status, role) = call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{guild_id}/roles"),
        &owner,
        Some(json!({"name": "mods", "permissions": "8192"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let role_id = role["id"].as_str().expect("role id").to_owned();

    let (status, _) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/members"),
        &guest,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "non-members cannot list members"
    );

    let (status, invite) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{channel_id}/invites"),
        &owner,
        Some(json!({"max_uses": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let code = invite["code"].as_str().expect("code").to_owned();

    let (status, joined) = call(
        &app,
        Method::POST,
        &format!("/api/v1/invites/{code}"),
        &guest,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "accepting the invite joins the guild"
    );
    assert_eq!(joined["guild"]["id"], guild_id);

    let (status, builtins) = call(&app, Method::GET, "/api/v1/stickers/packs", &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(builtins.as_array().unwrap().len(), 3);
    let builtin_path = format!(
        "/api/v1/stickers/{}/asset",
        builtins[0]["id"].as_str().unwrap()
    );
    let (status, asset) = send(&app, request(Method::GET, &builtin_path, None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(asset["w"], 320);
    let resolve_builtin = format!("/internal/v1/channels/{channel_id}/users/{owner_id}/stickers/1");
    let internal_request = axum::http::Request::builder()
        .method(Method::GET)
        .uri(&resolve_builtin)
        .header("x-internal-token", support::http::INTERNAL_TOKEN)
        .body(axum::body::Body::empty())
        .unwrap();
    let (status, resolved) = send(&app, internal_request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resolved["id"], "1");

    // A server manager can create, edit and remove a sticker; an ordinary member cannot.
    let path = format!("/api/v1/guilds/{guild_id}/stickers");
    let sticker_request = json!({"name": "wave", "tags": "wave", "image": "data:application/json;base64,eyJ2IjoiNS4wIiwidyI6MzIwLCJoIjozMjAsImZyIjozMCwiaXAiOjAsIm9wIjo2MCwibGF5ZXJzIjpbXX0="});
    let (status, _) = call(
        &app,
        Method::POST,
        &path,
        &guest,
        Some(sticker_request.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, sticker) = call(&app, Method::POST, &path, &owner, Some(sticker_request)).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(sticker["format_type"], 3);
    let sticker_path = format!("{path}/{}", sticker["id"].as_str().unwrap());
    let resolve_path = format!(
        "/internal/v1/channels/{channel_id}/users/{owner_id}/stickers/{}",
        sticker["id"].as_str().unwrap()
    );
    let (status, _) = send(&app, request(Method::GET, &resolve_path, None, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let internal_request = axum::http::Request::builder()
        .method(Method::GET)
        .uri(&resolve_path)
        .header("x-internal-token", support::http::INTERNAL_TOKEN)
        .body(axum::body::Body::empty())
        .unwrap();
    let (status, resolved) = send(&app, internal_request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resolved["id"], sticker["id"]);

    let (status, edited) = call(
        &app,
        Method::PATCH,
        &sticker_path,
        &owner,
        Some(json!({"name":"hello"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(edited["name"], "hello");
    let (status, listed) = call(&app, Method::GET, &path, &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let (status, _) = call(&app, Method::DELETE, &sticker_path, &owner, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, listed) = call(&app, Method::GET, &path, &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(listed.as_array().unwrap().is_empty());

    let (status, members) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/members?limit=1000"),
        &guest,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(members.as_array().map(Vec::len), Some(2));

    let (status, _) = call(
        &app,
        Method::PUT,
        &format!("/api/v1/guilds/{guild_id}/members/{guest_id}/roles/{role_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = call(
        &app,
        Method::PUT,
        &format!("/api/v1/guilds/{guild_id}/bans/{guest_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, bans) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/bans"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bans.as_array().map(Vec::len), Some(1));
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/v1/guilds/{guild_id}/bans/{guest_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, emoji) = call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{guild_id}/emojis"),
        &owner,
        Some(json!({"name": "wave", "image": "data:image/png;base64,iVBORw0KGgo=", "roles": []})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(emoji["name"], "wave");

    let (status, log) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/audit-logs"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        log["audit_log_entries"]
            .as_array()
            .map(|entries| !entries.is_empty())
            .unwrap_or(false),
        "mutations leave audit entries"
    );

    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/v1/guilds/{everyone_id}"),
        &guest,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "only the owner may delete a guild"
    );
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/v1/guilds/{guild_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}
