//! HTTP coverage for profile badge privacy and owner controls.
mod support;

use axum::http::{Method, StatusCode};
use interface::state::AppState;
use serde_json::json;
use std::sync::Arc;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database, Database};

fn app(database: &Database) -> axum::Router {
    let (private, public) = jwt_keys();
    interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    })
}

async fn register(app: &axum::Router, username: &str) -> (String, String) {
    let (status, body) = send(app, request(Method::POST, "/api/v1/auth/register", None, Some(json!({
        "email": format!("{username}@example.com"), "username": username, "password": "a-long-enough-passphrase"
    })))).await;
    assert_eq!(status, StatusCode::CREATED);
    (
        body["access_token"].as_str().unwrap().to_owned(),
        body["user"]["id"].as_str().unwrap().to_owned(),
    )
}

#[tokio::test]
async fn badge_visibility_is_private_and_cannot_grant_badges() {
    let database = start_database().await;
    let app = app(&database);
    let (alice, alice_id) = register(&app, "badgealice").await;
    let (bob, _) = register(&app, "badgebob").await;
    sqlx::query("INSERT INTO user_badges (user_id, badge_id) VALUES ($1, 'fixture_one'), ($1, 'fixture_two')")
        .bind(alice_id.parse::<i64>().unwrap()).execute(&database.pool).await.unwrap();
    let (status, _) = call(
        &app,
        Method::PUT,
        "/api/v1/users/@me/badges/fixture_one",
        &alice,
        Some(json!({"visible": false})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, own) = call(&app, Method::GET, "/api/v1/users/@me/badges", &alice, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        own,
        json!([{"id":"fixture_one","visible":false},{"id":"fixture_two","visible":true}])
    );
    let (status, public) = call(
        &app,
        Method::GET,
        &format!("/api/v1/users/{alice_id}/badges"),
        &bob,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(public, json!(["fixture_two"]));
    let (status, _) = call(
        &app,
        Method::PUT,
        "/api/v1/users/@me/badges/fixture_one",
        &bob,
        Some(json!({"visible": true})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, public) = call(
        &app,
        Method::GET,
        &format!("/api/v1/users/{alice_id}/badges"),
        &bob,
        None,
    )
    .await;
    assert_eq!(public, json!(["fixture_two"]));
}
