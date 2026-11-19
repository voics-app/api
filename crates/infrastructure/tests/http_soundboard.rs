mod support;
use axum::http::{Method, StatusCode};
use base64::Engine;
use interface::state::AppState;
use serde_json::json;
use support::http::{call, request, send};
use support::{jwt_keys, ports, start_database};

fn wav() -> Vec<u8> {
    let mut data = Vec::from(&b"RIFF"[..]);
    data.extend_from_slice(&38u32.to_le_bytes());
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16u32.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&48_000u32.to_le_bytes());
    data.extend_from_slice(&96_000u32.to_le_bytes());
    data.extend_from_slice(&2u16.to_le_bytes());
    data.extend_from_slice(&16u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&2u32.to_le_bytes());
    data.extend_from_slice(&[0, 0]);
    data
}
async fn register(app: &axum::Router, email: &str, username: &str) -> String {
    let (status, body) = send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({"email":email,"username":username,"password":"a-long-enough-passphrase"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    body["access_token"].as_str().unwrap().to_owned()
}
#[tokio::test]
async fn soundboard_crud_enforces_membership_and_validates_audio() {
    let db = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&db.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: Default::default(),
    });
    let owner = register(&app, "sound-owner@example.com", "soundowner").await;
    let outsider = register(&app, "sound-outsider@example.com", "outsider").await;
    let (_, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name":"Sounds"})),
    )
    .await;
    let guild = guild["id"].as_str().unwrap();
    let path = format!("/api/v1/guilds/{guild}/soundboard-sounds");
    let sound = base64::engine::general_purpose::STANDARD.encode(wav());
    let (status, body) = call(
        &app,
        Method::POST,
        &path,
        &owner,
        Some(json!({"name":"horn", "sound":sound})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["id"].as_str().unwrap();
    assert!(body.get("audio").is_none());
    let detail = format!("{path}/{id}");
    assert_eq!(
        call(&app, Method::GET, &detail, &outsider, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            Method::POST,
            &path,
            &owner,
            Some(json!({"name":"bad", "sound":"aGVsbG8="}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, updated) = call(
        &app,
        Method::PATCH,
        &detail,
        &owner,
        Some(json!({"name":"beep", "volume":0.5})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["name"], "beep");
    assert_eq!(
        call(&app, Method::DELETE, &detail, &owner, None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, Method::GET, &detail, &owner, None).await.0,
        StatusCode::NOT_FOUND
    );
}
