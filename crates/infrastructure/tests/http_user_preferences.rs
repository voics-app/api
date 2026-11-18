//! HTTP contract tests for synced `preferences` and `message_display_compact` (client#123).

mod support;

use axum::http::{Method, StatusCode};
use serde_json::json;

use interface::state::AppState;
use support::http::{INTERNAL_TOKEN, call, request, send};
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

    // The content preference follows this account and preserves unrelated preferences.
    let (status, hidden) = call(
        &app,
        Method::PATCH,
        url,
        &token,
        Some(json!({"preferences": {"hide_age_restricted_channels": true}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        hidden["preferences"]["hide_age_restricted_channels"],
        json!(true)
    );
    assert_eq!(hidden["preferences"]["accent"], json!("sky"));
    let (_, stored) = call(&app, Method::GET, url, &token, None).await;
    assert_eq!(stored["preferences"], hidden["preferences"]);

    let (_, other) = send(
        &app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({"email": "other-pref@example.com", "username": "other-pref", "password": "a-long-enough-passphrase"})),
        ),
    )
    .await;
    let other_token = other["access_token"].as_str().expect("other token");
    let (_, other_settings) = call(&app, Method::GET, url, other_token, None).await;
    assert!(
        other_settings["preferences"]
            .get("hide_age_restricted_channels")
            .is_none()
    );

    let (status, visible) = call(
        &app,
        Method::PATCH,
        url,
        &token,
        Some(json!({"preferences": {"hide_age_restricted_channels": false}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        visible["preferences"]["hide_age_restricted_channels"],
        json!(false)
    );
    let (_, settings) = call(&app, Method::GET, url, &token, None).await;
    assert_eq!(settings["preferences"], visible["preferences"]);

    for bad in [
        json!({"nope": true}),
        json!({"accent": "teal"}),
        json!({"font_scale": 9}),
        json!({"notify_sounds": "yes"}),
        json!({"hide_age_restricted_channels": "false"}),
        json!({"hide_age_restricted_channels": null}),
        json!({"hide_age_restricted_channels": 0}),
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
