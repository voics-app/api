//! HTTP contract tests for synced `preferences` and `message_display_compact` (client#123).

mod support;

use axum::http::{Method, StatusCode};
use serde_json::json;

use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database};

#[tokio::test]
async fn preferences_merge_round_trip_and_validate() {
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
            Some(json!({"email": "pref@example.com", "username": "pref", "password": "a-long-enough-passphrase"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let token = user["access_token"].as_str().expect("token").to_owned();
    let url = "/api/v1/users/@me/settings";

    let (_, settings) = call(&app, Method::GET, url, &token, None).await;
    assert_eq!(settings["preferences"], json!({}));
    assert_eq!(settings["message_display_compact"], json!(false));

    let (status, updated) = call(
        &app,
        Method::PATCH,
        url,
        &token,
        Some(json!({
            "message_display_compact": true,
            "preferences": {"accent": "sky", "notify_sounds": false, "sound_toggles": {"message": false}}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["message_display_compact"], json!(true));

    // A second patch merges key by key and leaves the rest alone.
    let (status, _) = call(
        &app,
        Method::PATCH,
        url,
        &token,
        Some(json!({"preferences": {"font_scale": 1.25}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, settings) = call(&app, Method::GET, url, &token, None).await;
    assert_eq!(
        settings["preferences"],
        json!({"accent": "sky", "notify_sounds": false, "sound_toggles": {"message": false}, "font_scale": 1.25})
    );
    assert_eq!(settings["message_display_compact"], json!(true));

    for bad in [
        json!({"nope": true}),
        json!({"accent": "teal"}),
        json!({"font_scale": 9}),
        json!({"notify_sounds": "yes"}),
    ] {
        let (status, _) = call(
            &app,
            Method::PATCH,
            url,
            &token,
            Some(json!({"preferences": bad})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let (_, after) = call(&app, Method::GET, url, &token, None).await;
    assert_eq!(after["preferences"], settings["preferences"]);
}
