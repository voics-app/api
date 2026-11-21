//! Stage instance lifecycle through the public API and its transactional outbox.
mod support;
use axum::http::{Method, StatusCode};
use interface::state::AppState;
use serde_json::{json, Value};
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database};

async fn register(app: &axum::Router, name: &str) -> String {
    let (status, body) = send(app, request(Method::POST, "/api/v1/auth/register", None,
        Some(json!({"email": format!("{name}@example.com"), "username": name, "password": "a-long-enough-passphrase"})))).await;
    assert_eq!(status, StatusCode::CREATED);
    body["access_token"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn stage_lifecycle_enforces_access_and_emits_outbox_events() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let owner = register(&app, "stageowner").await;
    let stranger = register(&app, "stagestranger").await;
    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name":"Stage guild"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild_id = guild["id"].as_str().unwrap();
    let (status, channel) = call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        Some(json!({"name":"town-hall", "type":13})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{channel}");
    let channel_id = channel["id"].as_str().unwrap();
    let path = format!("/api/v1/stage-instances/{channel_id}");
    let body = json!({"channel_id":channel_id, "topic":"Town hall", "privacy_level":2});
    assert_eq!(
        call(
            &app,
            Method::POST,
            "/api/v1/stage-instances",
            &stranger,
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, stage) = call(
        &app,
        Method::POST,
        "/api/v1/stage-instances",
        &owner,
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{stage}");
    assert_eq!(stage["topic"], "Town hall");
    assert_eq!(
        call(
            &app,
            Method::POST,
            "/api/v1/stage-instances",
            &owner,
            Some(body)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(&app, Method::GET, &path, &stranger, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(call(&app, Method::GET, &path, &owner, None).await.1, stage);
    let (status, updated) = call(
        &app,
        Method::PATCH,
        &path,
        &owner,
        Some(json!({"topic":"Questions"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["topic"], "Questions");
    assert_eq!(
        call(&app, Method::DELETE, &path, &owner, None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, Method::GET, &path, &owner, None).await.0,
        StatusCode::NOT_FOUND
    );
    let events: Vec<(Value,)> = sqlx::query_as(
        "SELECT payload FROM outbox WHERE payload->>'type' LIKE 'STAGE_INSTANCE_%' ORDER BY id",
    )
    .fetch_all(&database.pool)
    .await
    .unwrap();
    let kinds: Vec<&str> = events
        .iter()
        .map(|(event,)| event["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "STAGE_INSTANCE_CREATE",
            "STAGE_INSTANCE_UPDATE",
            "STAGE_INSTANCE_DELETE"
        ]
    );
    assert_eq!(events[2].0["data"]["topic"], "Questions");
}
