//! HTTP contract test for `GET /users/{id}/profile`: mutual servers and mutual friends.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send};
use support::{jwt_keys, ports, start_database};

async fn register(app: &axum::Router, username: &str) -> (String, String) {
    let (status, body) = send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({"email": format!("{username}@example.com"), "username": username, "password": "a-long-enough-passphrase"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "register {username}");
    (
        body["access_token"].as_str().expect("token").to_owned(),
        body["user"]["id"].as_str().expect("id").to_owned(),
    )
}

async fn befriend(app: &axum::Router, from: &str, to: &str, to_token: &str, from_id: &str) {
    let (status, _) = call(
        app,
        Method::POST,
        "/api/v1/users/@me/relationships",
        from,
        Some(json!({"username": to})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        app,
        Method::PUT,
        &format!("/api/v1/users/@me/relationships/{from_id}"),
        to_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn profile_lists_mutual_guilds_and_mutual_friends() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let (alice, alice_id) = register(&app, "alice").await;
    let (bob, bob_id) = register(&app, "bob").await;
    let (carol, carol_id) = register(&app, "carol").await;

    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &alice,
        Some(json!({"name": "Shared"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild_id = guild["id"].as_str().expect("guild id").to_owned();
    let (_, channels) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &alice,
        None,
    )
    .await;
    let channel_id = channels[0]["id"].as_str().expect("channel").to_owned();
    let (_, invite) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{channel_id}/invites"),
        &alice,
        Some(json!({})),
    )
    .await;
    let code = invite["code"].as_str().expect("code").to_owned();
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/v1/invites/{code}"),
        &bob,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // carol is a friend of both alice and bob.
    befriend(&app, &alice, "carol", &carol, &alice_id).await;
    befriend(&app, &bob, "carol", &carol, &bob_id).await;

    let (status, profile) = call(
        &app,
        Method::GET,
        &format!("/api/v1/users/{bob_id}/profile"),
        &alice,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(profile["user"]["username"], "bob");
    assert!(profile["user"].get("email").is_none());
    let guilds = profile["mutual_guilds"].as_array().expect("guilds");
    assert_eq!(guilds.len(), 1);
    assert_eq!(guilds[0]["id"], guild_id);
    let friends = profile["mutual_friends"].as_array().expect("friends");
    assert_eq!(friends.len(), 1);
    assert_eq!(friends[0]["id"], carol_id);

    // Carol shares no server with alice, and alice and carol have no mutual friend.
    let (status, profile) = call(
        &app,
        Method::GET,
        &format!("/api/v1/users/{carol_id}/profile"),
        &alice,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(profile["mutual_guilds"], json!([]));
    assert_eq!(profile["mutual_friends"], json!([]));

    // Own profile has no mutuals; unknown users 404.
    let (status, own) = call(
        &app,
        Method::GET,
        &format!("/api/v1/users/{alice_id}/profile"),
        &alice,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(own["mutual_guilds"], Value::Array(vec![]));
    let (status, _) = call(
        &app,
        Method::GET,
        "/api/v1/users/999999999999999999/profile",
        &alice,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn profile_customization_persists_public_fields_and_supports_reset() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let (alice, alice_id) = register(&app, "alice").await;
    let (bob, _) = register(&app, "bob").await;
    let (status, updated) = call(
        &app,
        Method::PATCH,
        "/api/v1/users/@me",
        &alice,
        Some(
            json!({"pronouns":"  they / them  ","theme_colors":[0,16777215],"accent_color":0,
            "banner":"data:image/png;base64,aW1hZ2U=","bio":"Global bio"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["pronouns"], "they / them");
    assert_eq!(updated["theme_colors"], json!([0, 16777215]));
    assert_eq!(updated["accent_color"], 0);
    assert!(updated["banner"]
        .as_str()
        .is_some_and(|hash| !hash.is_empty() && !hash.starts_with("data:")));
    let (_, public) = call(
        &app,
        Method::GET,
        &format!("/api/v1/users/{alice_id}"),
        &bob,
        None,
    )
    .await;
    for field in ["pronouns", "theme_colors", "accent_color", "banner"] {
        assert_eq!(public[field], updated[field], "public {field}");
    }
    assert!(public.get("email").is_none());
    assert!(public.get("mfa_enabled").is_none());
    let (_, preserved) = call(
        &app,
        Method::PATCH,
        "/api/v1/users/@me",
        &alice,
        Some(json!({"global_name":"Alice"})),
    )
    .await;
    assert_eq!(preserved["theme_colors"], updated["theme_colors"]);
    assert_eq!(preserved["banner"], updated["banner"]);
    let (_, reset) = call(
        &app,
        Method::PATCH,
        "/api/v1/users/@me",
        &alice,
        Some(json!({"pronouns":" ","theme_colors":null,"accent_color":null,"banner":null})),
    )
    .await;
    for field in ["pronouns", "theme_colors", "accent_color", "banner"] {
        assert!(reset[field].is_null(), "reset {field}: {reset}");
    }
    assert_eq!(reset["bio"], "Global bio");
}
