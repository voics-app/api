//! HTTP contract tests for `pinned_dm_channel_ids` / `pinned_channel_ids` in user settings (client#222, client#223).

mod support;

use axum::http::{Method, StatusCode};
use serde_json::json;

use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database};

#[tokio::test]
async fn pins_round_trip_and_validate() {
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
            Some(json!({"email": "pin@example.com", "username": "pinner", "password": "a-long-enough-passphrase"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let token = user["access_token"].as_str().expect("token").to_owned();
    let url = "/api/v1/users/@me/settings";

    let (_, settings) = call(&app, Method::GET, url, &token, None).await;
    assert_eq!(settings["pinned_dm_channel_ids"], json!([]));
    assert_eq!(settings["pinned_channel_ids"], json!({}));

    let channels = json!({"500": ["11", "12"], "600": ["11"]});
    let (status, updated) = call(
        &app,
        Method::PATCH,
        url,
        &token,
        Some(json!({"pinned_dm_channel_ids": ["1", "2", "3"], "pinned_channel_ids": channels})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["pinned_dm_channel_ids"], json!(["1", "2", "3"]));

    let (_, settings) = call(&app, Method::GET, url, &token, None).await;
    assert_eq!(settings["pinned_dm_channel_ids"], json!(["1", "2", "3"]));
    assert_eq!(settings["pinned_channel_ids"], channels);

    let (_, settings) = call(
        &app,
        Method::PATCH,
        url,
        &token,
        Some(json!({"pinned_channel_ids": {"500": [], "600": ["11"]}})),
    )
    .await;
    assert_eq!(settings["pinned_channel_ids"], json!({"600": ["11"]}));
    assert_eq!(settings["pinned_dm_channel_ids"], json!(["1", "2", "3"]));

    let eleven: Vec<String> = (1..=11).map(|n| n.to_string()).collect();
    for bad in [
        json!({"pinned_dm_channel_ids": ["1", "2", "3", "4", "5", "6"]}),
        json!({"pinned_dm_channel_ids": ["1", "1"]}),
        json!({"pinned_channel_ids": {"500": eleven}}),
        json!({"pinned_channel_ids": {"500": ["1", "1"]}}),
    ] {
        let (status, _) = call(&app, Method::PATCH, url, &token, Some(bad)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    let (_, settings) = call(&app, Method::GET, url, &token, None).await;
    assert_eq!(settings["pinned_dm_channel_ids"], json!(["1", "2", "3"]));
}
