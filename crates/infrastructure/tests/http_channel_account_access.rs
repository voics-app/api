//! Current account eligibility must be checked even for offline authors and guild owners.
mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    Router,
};
use interface::state::{AppState, RatePolicy};
use serde_json::{json, Value};
use std::sync::Arc;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database};

async fn register(app: &Router, name: &str) -> (String, String) {
    let (status, body) = send(app, request(Method::POST, "/api/v1/auth/register", None,
        Some(json!({"email":format!("{name}@example.com"), "username":name, "password":"a-long-enough-passphrase"})))).await;
    assert_eq!(status, StatusCode::CREATED);
    (
        body["access_token"].as_str().unwrap().into(),
        body["user"]["id"].as_str().unwrap().into(),
    )
}

async fn access(app: &Router, channel: &str, user: &str) -> (StatusCode, Value) {
    send(
        app,
        Request::builder()
            .uri(format!("/internal/v1/channels/{channel}/access/{user}"))
            .header("x-internal-token", INTERNAL_TOKEN)
            .body(Body::empty())
            .unwrap(),
    )
    .await
}

#[tokio::test]
async fn offline_account_eligibility_is_rechecked_for_guilds_and_direct_messages() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: Arc::new(support::AllowAll),
        rate_policy: RatePolicy::default(),
    });
    let (token, author) = register(&app, "schedule_author").await;
    let (_, peer) = register(&app, "schedule_peer").await;
    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &token,
        Some(json!({"name":"Schedules"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, channels) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{}/channels", guild["id"].as_str().unwrap()),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let guild_channel = channels
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["type"] == 0)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, dm) = call(
        &app,
        Method::POST,
        "/api/v1/users/@me/channels",
        &token,
        Some(json!({"recipient_ids":[peer]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let direct = dm["id"].as_str().unwrap();
    let id: i64 = author.parse().unwrap();
    sqlx::query("UPDATE sessions SET revoked_at=now() WHERE user_id=$1")
        .bind(id)
        .execute(&database.pool)
        .await
        .unwrap();
    for channel in [&guild_channel, direct] {
        assert_eq!(
            access(&app, channel, &author).await.0,
            StatusCode::OK,
            "offline eligible author"
        );
    }

    // Every case starts from an eligible account, then reads through the real HTTP/PG boundary.
    for (name, mutation, expected) in [
        (
            "disabled",
            "UPDATE users SET disabled_at=now() WHERE id=$1",
            StatusCode::FORBIDDEN,
        ),
        (
            "pending deletion",
            "UPDATE users SET delete_after=now()+interval '14 days' WHERE id=$1",
            StatusCode::FORBIDDEN,
        ),
        (
            "indefinite suspension",
            "UPDATE users SET suspended=true WHERE id=$1",
            StatusCode::FORBIDDEN,
        ),
        (
            "future suspension",
            "UPDATE users SET suspended=true,suspended_until=now()+interval '1 hour' WHERE id=$1",
            StatusCode::FORBIDDEN,
        ),
        (
            "expired suspension",
            "UPDATE users SET suspended=true,suspended_until=now()-interval '1 hour' WHERE id=$1",
            StatusCode::OK,
        ),
        (
            "eligible bot",
            "UPDATE users SET bot=true WHERE id=$1",
            StatusCode::OK,
        ),
    ] {
        sqlx::query("UPDATE users SET disabled_at=NULL,delete_after=NULL,suspended=false,suspended_until=NULL,bot=false WHERE id=$1")
            .bind(id).execute(&database.pool).await.unwrap();
        sqlx::query(mutation)
            .bind(id)
            .execute(&database.pool)
            .await
            .unwrap();
        for channel in [&guild_channel, direct] {
            let (status, body) = access(&app, channel, &author).await;
            assert_eq!(status, expected, "{name} in {channel}");
            if expected == StatusCode::FORBIDDEN {
                assert_eq!(body["code"], 50001);
            }
        }
    }
    sqlx::query("UPDATE users SET disabled_at=NULL,delete_after=NULL,suspended=false,suspended_until=NULL,bot=false WHERE id=$1")
        .bind(id).execute(&database.pool).await.unwrap();
    assert_eq!(
        access(&app, &guild_channel, &author).await.0,
        StatusCode::OK,
        "reactivation"
    );
    assert_eq!(
        access(&app, &guild_channel, &peer).await.0,
        StatusCode::FORBIDDEN,
        "eligible non-member"
    );
    assert_eq!(
        access(&app, "1", &author).await.0,
        StatusCode::NOT_FOUND,
        "eligible author / missing channel"
    );
}

#[tokio::test]
async fn a_deleted_account_is_denied_while_its_former_dm_still_exists() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: Arc::new(support::AllowAll),
        rate_policy: RatePolicy::default(),
    });
    let (token, author) = register(&app, "missing_author").await;
    let (_, peer) = register(&app, "missing_peer").await;
    let (_, dm) = call(
        &app,
        Method::POST,
        "/api/v1/users/@me/channels",
        &token,
        Some(json!({"recipient_ids":[peer]})),
    )
    .await;
    let channel = dm["id"].as_str().unwrap();
    sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(author.parse::<i64>().unwrap())
        .execute(&database.pool)
        .await
        .unwrap();
    assert_eq!(access(&app, channel, &peer).await.0, StatusCode::OK);
    let (status, body) = access(&app, channel, &author).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], 50001);
}
