//! New guild-related DM initiation and per-server preference resolution.
mod support;
use axum::http::{Method, StatusCode};
use interface::state::AppState;
use serde_json::{json, Value};
use support::http::{call, request, send};
use support::{jwt_keys, ports, start_database};

async fn register(app: &axum::Router, name: &str) -> (String, String) {
    let (status, body) = send(app, request(Method::POST, "/api/v1/auth/register", None, Some(json!({"email": format!("{name}@example.com"), "username": name, "password": "a-long-enough-passphrase"})))).await;
    assert_eq!(status, StatusCode::CREATED);
    (
        body["access_token"].as_str().unwrap().to_owned(),
        body["user"]["id"].as_str().unwrap().to_owned(),
    )
}
async fn guild(app: &axum::Router, owner: &str, guests: &[&str]) -> String {
    let (status, g) = call(
        app,
        Method::POST,
        "/api/v1/guilds",
        owner,
        Some(json!({"name":"Shared server"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = g["id"].as_str().unwrap().to_owned();
    let (_, channels) = call(
        app,
        Method::GET,
        &format!("/api/v1/guilds/{id}/channels"),
        owner,
        None,
    )
    .await;
    let channel = channels[0]["id"].as_str().unwrap();
    let (status, invite) = call(
        app,
        Method::POST,
        &format!("/api/v1/channels/{channel}/invites"),
        owner,
        Some(json!({"max_uses":0})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for guest in guests {
        let (status, _) = call(
            app,
            Method::POST,
            &format!("/api/v1/invites/{}", invite["code"].as_str().unwrap()),
            guest,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    id
}
async fn settings(app: &axum::Router, token: &str, preferences: Value) {
    let (status, _) = call(
        app,
        Method::PATCH,
        "/api/v1/users/@me/settings",
        token,
        Some(json!({"preferences":preferences})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}
async fn open(app: &axum::Router, token: &str, ids: &[&str]) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/api/v1/users/@me/channels",
        token,
        Some(json!({"recipient_ids": ids})),
    )
    .await
}

#[tokio::test]
async fn new_guild_dms_resolve_preferences_and_preserve_existing_conversations() {
    let db = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&db.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let (alice, alice_id) = register(&app, "alice").await;
    let (bob, bob_id) = register(&app, "bob").await;
    let (carol, carol_id) = register(&app, "carol").await;
    let (dave, dave_id) = register(&app, "dave").await;
    let (eve, eve_id) = register(&app, "eve").await;
    let (frank, _frank_id) = register(&app, "frank").await;
    let (george, _) = register(&app, "george").await;
    let (status, existing) = open(&app, &carol, &[&bob_id]).await;
    assert_eq!(status, StatusCode::OK);
    let one = guild(&app, &bob, &[&alice, &carol, &dave, &eve, &frank]).await;
    settings(
        &app,
        &bob,
        json!({"guild_dm_restrictions": {(one.clone()): true}}),
    )
    .await;
    assert_eq!(
        open(&app, &alice, &[&bob_id]).await.0,
        StatusCode::FORBIDDEN,
        "a server override restricts even with permissive default"
    );
    assert_eq!(
        open(&app, &george, &[&bob_id]).await.0,
        StatusCode::OK,
        "no shared server retains direct access"
    );
    settings(&app, &bob, json!({"default_guilds_restricted": true})).await;
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/users/@me/relationships",
        &frank,
        Some(json!({"username":"bob"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/users/@me/relationships",
        &bob,
        Some(json!({"username":"frank"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        open(&app, &frank, &[&bob_id]).await.0,
        StatusCode::OK,
        "friends can initiate"
    );
    let (status, denied) = open(&app, &alice, &[&bob_id]).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(denied["code"], 50007);
    let (status, reused) = open(&app, &carol, &[&bob_id]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reused["id"], existing["id"]);
    let (status, _) = open(&app, &alice, &[&bob_id, &dave_id]).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a group cannot bypass the policy"
    );
    let (status, group) = open(&app, &alice, &[&dave_id, &eve_id]).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &app,
        Method::PUT,
        &format!(
            "/api/v1/channels/{}/recipients/{bob_id}",
            group["id"].as_str().unwrap()
        ),
        &alice,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "adding to a group cannot bypass the policy"
    );
    settings(
        &app,
        &bob,
        json!({"guild_dm_restrictions": {(one.clone()): false}}),
    )
    .await;
    assert_eq!(
        open(&app, &alice, &[&bob_id]).await.0,
        StatusCode::OK,
        "server override allows a new DM"
    );
    settings(
        &app,
        &bob,
        json!({"guild_dm_restrictions": {(one.clone()): true}}),
    )
    .await;
    // A second shared server with no override inherits the restricted default.
    let two = guild(&app, &bob, &[&dave]).await;
    assert_eq!(open(&app, &dave, &[&bob_id]).await.0, StatusCode::FORBIDDEN);
    settings(
        &app,
        &bob,
        json!({"guild_dm_restrictions": {(two.clone()): false}}),
    )
    .await;
    assert_eq!(
        open(&app, &dave, &[&bob_id]).await.0,
        StatusCode::OK,
        "one shared server allows initiation"
    );
    // Alice's existing conversation remains available even after restriction is restored.
    assert_eq!(open(&app, &alice, &[&bob_id]).await.0, StatusCode::OK);
    let _ = (alice_id, carol_id, eve_id);
}
