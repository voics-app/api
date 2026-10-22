//! HTTP contract tests for `guild_folders` / `guild_positions` in user settings (client#58).

mod support;

use axum::http::{Method, StatusCode};
use serde_json::json;

use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database};

#[tokio::test]
async fn guild_folders_round_trip_and_validate() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let state = AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    };
    let app = interface::router(state);
    let (status, user) = send(
        &app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({"email": "dora@example.com", "username": "dora", "password": "a-long-enough-passphrase"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let token = user["access_token"].as_str().expect("token").to_owned();
    let url = "/api/v1/users/@me/settings";

    let (_, settings) = call(&app, Method::GET, url, &token, None).await;
    assert_eq!(settings["guild_folders"], json!([]));
    assert_eq!(settings["guild_positions"], json!([]));

    let folders = json!([
        {"id": 7, "name": "Work", "color": 0x336699, "guild_ids": ["101", "102"]},
        {"id": null, "name": null, "color": null, "guild_ids": ["103"]}
    ]);
    let (status, updated) = call(
        &app,
        Method::PATCH,
        url,
        &token,
        Some(json!({"guild_folders": folders, "guild_positions": ["101", "102", "103"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["guild_folders"], folders);

    let (_, settings) = call(&app, Method::GET, url, &token, None).await;
    assert_eq!(settings["guild_folders"], folders);
    assert_eq!(settings["guild_positions"], json!(["101", "102", "103"]));

    for bad in [
        json!([{"id": 1, "guild_ids": ["1"]}, {"id": 2, "guild_ids": ["1"]}]),
        json!([{"id": null, "guild_ids": ["1", "2"]}]),
        json!([{"id": 1, "guild_ids": []}]),
        json!([{"id": 1, "name": "x".repeat(33), "guild_ids": ["1"]}]),
    ] {
        let (status, _) = call(
            &app,
            Method::PATCH,
            url,
            &token,
            Some(json!({"guild_folders": bad})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
