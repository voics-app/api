//! Friend request source policies at both public request routes.
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

async fn settings(app: &axum::Router, token: &str, flags: Value) {
    let (status, _) = call(
        app,
        Method::PATCH,
        "/api/v1/users/@me/settings",
        token,
        Some(json!({"friend_source_flags": flags})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}
async fn request_id(app: &axum::Router, token: &str, id: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::PUT,
        &format!("/api/v1/users/@me/relationships/{id}"),
        token,
        Some(json!({"type": 1})),
    )
    .await
}
async fn request_name(app: &axum::Router, token: &str, name: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/api/v1/users/@me/relationships",
        token,
        Some(json!({"username": name})),
    )
    .await
}
#[tokio::test]
async fn sources_gate_new_requests_but_not_acceptance_or_blocks() {
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
    let (dave, _dave_id) = register(&app, "dave").await;
    let (eve, eve_id) = register(&app, "eve").await;
    let (frank, frank_id) = register(&app, "frank").await;
    let (_, initial) = call(&app, Method::GET, "/api/v1/users/@me/settings", &bob, None).await;
    assert_eq!(
        initial["friend_source_flags"],
        json!({"all": true, "mutual_friends": false, "mutual_guilds": false})
    );
    // Bob initiates before restricting, and Alice can still accept afterward.
    assert_eq!(
        request_id(&app, &bob, &alice_id).await.0,
        StatusCode::NO_CONTENT
    );
    settings(&app, &bob, json!({"all": false})).await;
    assert_eq!(
        request_id(&app, &alice, &bob_id).await.0,
        StatusCode::NO_CONTENT
    );
    let (status, denied) = request_id(&app, &carol, &bob_id).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(denied["code"], 50007);
    assert_eq!(
        request_name(&app, &dave, "bob").await.0,
        StatusCode::FORBIDDEN
    );
    let (_, rows) = call(
        &app,
        Method::GET,
        "/api/v1/users/@me/relationships",
        &bob,
        None,
    )
    .await;
    assert!(!rows
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["user"]["id"] == carol_id));
    guild(&app, &bob, &[&carol]).await;
    settings(&app, &bob, json!({"mutual_guilds": true})).await;
    assert_eq!(
        request_name(&app, &carol, "bob").await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request_name(&app, &dave, "bob").await.0,
        StatusCode::FORBIDDEN
    );
    // Bob and Dave both befriend Frank. Bob can initiate regardless of his own incoming policy.
    assert_eq!(
        request_id(&app, &bob, &frank_id).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request_id(&app, &frank, &bob_id).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request_id(&app, &dave, &frank_id).await.0,
        StatusCode::NO_CONTENT
    );
    let (_, dave_user) = call(&app, Method::GET, "/api/v1/users/@me", &dave, None).await;
    assert_eq!(
        request_id(&app, &frank, dave_user["id"].as_str().unwrap())
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    settings(
        &app,
        &bob,
        json!({"mutual_guilds": false, "mutual_friends": true}),
    )
    .await;
    assert_eq!(
        request_name(&app, &dave, "bob").await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request_id(&app, &eve, &bob_id).await.0,
        StatusCode::FORBIDDEN
    );
    // Blocking remains available, and it wins over source eligibility.
    assert_eq!(
        call(
            &app,
            Method::PUT,
            &format!("/api/v1/users/@me/relationships/{eve_id}"),
            &bob,
            Some(json!({"type": 2}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    settings(&app, &bob, json!({"all": true})).await;
    assert_eq!(
        request_id(&app, &eve, &bob_id).await.0,
        StatusCode::FORBIDDEN
    );
    let (_, final_settings) =
        call(&app, Method::GET, "/api/v1/users/@me/settings", &bob, None).await;
    assert_eq!(
        final_settings["friend_source_flags"],
        json!({"all": true, "mutual_friends": true, "mutual_guilds": false})
    );
}
