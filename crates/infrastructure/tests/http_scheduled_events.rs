//! HTTP reads enforce guild membership for scheduled events.
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
    let raw_guild_id: i64 = guild_id.parse().unwrap();
    sqlx::query("INSERT INTO guild_scheduled_events (id, guild_id, name, scheduled_start_time, scheduled_end_time, status, entity_type, location) VALUES (20, $1, 'Meetup', now() + interval '1 hour', now() + interval '2 hours', 1, 3, 'Park')")
        .bind(raw_guild_id).execute(&database.pool).await.unwrap();
    let path = format!("/api/v1/guilds/{guild_id}/scheduled-events");
    let (status, events) = call(&app, Method::GET, &path, &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(events[0]["name"], "Meetup");
    assert_eq!(events[0]["entity_metadata"]["location"], "Park");
    let (status, _) = call(&app, Method::GET, &path, &outsider, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, interest) = call(
        &app,
        Method::GET,
        &format!("{path}/20/users/@me"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(interest["interested"], false);
}
