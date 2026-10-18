//! Body limits for data-URI image uploads: image routes accept payloads up to
//! the image bound, every other route keeps axum's 2 MiB default.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::json;

use interface::state::AppState;
use support::http::{call, request, send};
use support::{jwt_keys, ports, start_database};

const MIB: usize = 1024 * 1024;

/// A `data:` URI whose decoded payload is `bytes` long (rounded down to a
/// multiple of three), built from `AAAA` groups so no base64 crate is needed.
fn data_uri(content_type: &str, bytes: usize) -> String {
    let groups = bytes / 3;
    format!("data:{content_type};base64,{}", "AAAA".repeat(groups))
}

#[tokio::test]
async fn image_routes_accept_seven_mib_and_reject_nine_mib_with_400() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });

    let (status, registered) = send(
        &app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({"email": "uploader@example.com", "username": "uploader", "password": "a-long-enough-passphrase"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let token = registered["access_token"]
        .as_str()
        .expect("token")
        .to_owned();

    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &token,
        Some(json!({"name": "Upload HQ"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild_path = format!("/api/v1/guilds/{}", guild["id"].as_str().expect("guild id"));

    let (status, _) = call(
        &app,
        Method::PATCH,
        &guild_path,
        &token,
        Some(json!({"icon": data_uri("image/gif", 7 * MIB)})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a 7 MiB GIF icon is accepted");

    let (status, body) = call(
        &app,
        Method::PATCH,
        &guild_path,
        &token,
        Some(json!({"icon": data_uri("image/gif", 9 * MIB)})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a 9 MiB icon is a validation error, not a dropped connection"
    );
    assert!(body.to_string().contains("BINARY_TYPE_MAX_SIZE"), "{body}");

    let (status, _) = call(
        &app,
        Method::PATCH,
        "/api/v1/users/@me",
        &token,
        Some(json!({"avatar": data_uri("image/png", 7 * MIB)})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a 7 MiB avatar is accepted on users/@me"
    );
}

#[tokio::test]
async fn non_image_routes_keep_the_two_mib_limit() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });

    let padding = "x".repeat(3 * MIB);
    let (status, body) = send(
        &app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({"email": "big@example.com", "username": "big", "password": padding})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.to_string().contains("INVALID_BODY"), "{body}");
}
