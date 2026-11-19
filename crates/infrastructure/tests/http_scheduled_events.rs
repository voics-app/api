//! HTTP creation and reads enforce guild membership for scheduled events.
mod support;

use axum::http::{Method, StatusCode};
use interface::state::AppState;
use serde_json::json;
use support::http::{call, request, send};
use support::{jwt_keys, ports, start_database};

async fn register(app: &axum::Router, name: &str) -> String {
    let (status, body) = send(app, request(Method::POST, "/api/v1/auth/register", None,
        Some(json!({"email": format!("{name}@example.com"), "username": name, "password": "a-long-enough-passphrase"})))).await;
    assert_eq!(status, StatusCode::CREATED);
    body["access_token"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn only_members_can_read_external_events_and_their_own_interest() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let owner = register(&app, "event_owner").await;
    let outsider = register(&app, "event_outsider").await;
    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name": "Meetups"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild_id = guild["id"].as_str().unwrap();
    let path = format!("/api/v1/guilds/{guild_id}/scheduled-events");
    let start = chrono::Utc::now() + chrono::Duration::hours(1);
    let end = start + chrono::Duration::hours(1);
    let input = json!({"name":"Meetup", "entity_type":3, "entity_metadata":{"location":"Park"}, "scheduled_start_time": start.to_rfc3339(), "scheduled_end_time":end.to_rfc3339()});
    let (status, _) = call(&app, Method::POST, &path, &outsider, Some(input.clone())).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, created) = call(&app, Method::POST, &path, &owner, Some(input)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let event_id = created["id"].as_str().unwrap();
    let (status, events) = call(&app, Method::GET, &path, &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(events[0]["name"], "Meetup");
    assert_eq!(events[0]["entity_metadata"]["location"], "Park");
    let (status, _) = call(&app, Method::GET, &path, &outsider, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, interest) = call(
        &app,
        Method::GET,
        &format!("{path}/{event_id}/users/@me"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(interest["interested"], false);
    let interest_path = format!("{path}/{event_id}/users/@me");
    for _ in 0..2 {
        let (status, _) = call(&app, Method::PUT, &interest_path, &owner, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    let (_, interest) = call(&app, Method::GET, &interest_path, &owner, None).await;
    assert_eq!(interest["interested"], true);
    let (_, events) = call(&app, Method::GET, &path, &owner, None).await;
    assert_eq!(events[0]["user_count"], 1);
    let added: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE payload->>'type' = 'GUILD_SCHEDULED_EVENT_USER_ADD' AND payload->'data'->>'guild_scheduled_event_id' = $1")
        .bind(event_id).fetch_one(&database.pool).await.unwrap();
    assert_eq!(added, 1);
    for _ in 0..2 {
        let (status, _) = call(&app, Method::DELETE, &interest_path, &owner, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    let removed: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE payload->>'type' = 'GUILD_SCHEDULED_EVENT_USER_REMOVE' AND payload->'data'->>'guild_scheduled_event_id' = $1")
        .bind(event_id).fetch_one(&database.pool).await.unwrap();
    assert_eq!(removed, 1);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE payload->>'type' = 'GUILD_SCHEDULED_EVENT_CREATE' AND payload->'data'->>'id' = $1")
        .bind(event_id).fetch_one(&database.pool).await.unwrap();
    assert_eq!(count, 1);
}
