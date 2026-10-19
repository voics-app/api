//! HTTP contract tests for the account-wide `e2ee_calls_default` setting (voice#8): the default,
//! its round trip through `PATCH /users/@me/settings`, and its presence in the internal READY
//! payload the gateway reads.

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database};

#[tokio::test]
async fn e2ee_calls_default_defaults_to_off_and_round_trips() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let state = AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    };
    let app = interface::router(state);

    let (status, alice) = send(
        &app,
        request(
            axum::http::Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({"email": "carol@example.com", "username": "carol", "password": "a-long-enough-passphrase"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let token = alice["access_token"].as_str().expect("token").to_owned();
    let user_id = alice["user"]["id"].as_str().expect("id").to_owned();

    let (status, settings) = call(
        &app,
        axum::http::Method::GET,
        "/api/v1/users/@me/settings",
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settings["e2ee_calls_default"], json!(false));

    let (status, updated) = call(
        &app,
        axum::http::Method::PATCH,
        "/api/v1/users/@me/settings",
        &token,
        Some(json!({"e2ee_calls_default": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["e2ee_calls_default"], json!(true));

    let (status, settings) = call(
        &app,
        axum::http::Method::GET,
        "/api/v1/users/@me/settings",
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settings["e2ee_calls_default"], json!(true));
    assert_eq!(settings["default_delivery_mode"], json!("standard"));

    let (status, ready) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!("/internal/v1/users/{user_id}/ready"))
            .header("x-internal-token", INTERNAL_TOKEN)
            .body(Body::empty())
            .expect("request"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ready_settings: &Value = &ready["user_settings"];
    assert_eq!(ready_settings["e2ee_calls_default"], json!(true));

    let (status, cleared) = call(
        &app,
        axum::http::Method::PATCH,
        "/api/v1/users/@me/settings",
        &token,
        Some(json!({"e2ee_calls_default": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cleared["e2ee_calls_default"], json!(false));
}
