//! HTTP contract tests for voice moderation through `PATCH /guilds/{id}/members/{uid}`:
//! moving a member to another voice channel and disconnecting them.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send};
use support::{jwt_keys, ports, start_database};

async fn register(app: &axum::Router, email: &str, username: &str) -> (String, String) {
    let (status, body) = send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({"email": email, "username": username, "password": "a-long-enough-passphrase"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "register {username}");
    (
        body["access_token"].as_str().expect("token").to_owned(),
        body["user"]["id"].as_str().expect("id").to_owned(),
    )
}

async fn create_channel(app: &axum::Router, token: &str, guild_id: &str, kind: u8) -> String {
    let (status, channel) = call(
        app,
        Method::POST,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        token,
        Some(json!({"name": "room", "type": kind})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    channel["id"].as_str().expect("channel id").to_owned()
}

fn audit_codes(log: &Value) -> Vec<u64> {
    log["audit_log_entries"]
        .as_array()
        .expect("entries")
        .iter()
        .filter_map(|entry| entry["action_type"].as_u64())
        .collect()
}

#[tokio::test]
async fn moderators_move_and_disconnect_members() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let (owner, _owner_id) = register(&app, "owner@example.com", "owner").await;
    let (guest, guest_id) = register(&app, "guest@example.com", "guest").await;

    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name": "Voice HQ"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild_id = guild["id"].as_str().expect("guild id").to_owned();
    let voice = create_channel(&app, &owner, &guild_id, 2).await;
    let text = create_channel(&app, &owner, &guild_id, 0).await;

    let (status, invite) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{text}/invites"),
        &owner,
        Some(json!({"max_uses": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let code = invite["code"].as_str().expect("code").to_owned();
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/v1/invites/{code}"),
        &guest,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let member_path = format!("/api/v1/guilds/{guild_id}/members/{guest_id}");

    let (status, _) = call(
        &app,
        Method::PATCH,
        &member_path,
        &guest,
        Some(json!({"channel_id": voice})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "moving needs MOVE_MEMBERS");
    let (status, _) = call(
        &app,
        Method::PATCH,
        &member_path,
        &guest,
        Some(json!({"channel_id": null})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "disconnecting needs MOVE_MEMBERS"
    );
    let (status, _) = call(
        &app,
        Method::PATCH,
        &member_path,
        &guest,
        Some(json!({"suppress": false})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "promotion needs MUTE_MEMBERS"
    );

    let (status, _) = call(
        &app,
        Method::PATCH,
        &member_path,
        &owner,
        Some(json!({"channel_id": text})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "the target must be a voice channel"
    );
    let (status, _) = call(
        &app,
        Method::PATCH,
        &member_path,
        &owner,
        Some(json!({"channel_id": "1"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the target must exist in this guild"
    );

    let (status, _) = call(
        &app,
        Method::PATCH,
        &member_path,
        &owner,
        Some(json!({"channel_id": voice})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "move to a voice channel");
    let (status, _) = call(
        &app,
        Method::PATCH,
        &member_path,
        &owner,
        Some(json!({"channel_id": null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "disconnect");
    let (status, _) = call(
        &app,
        Method::PATCH,
        &member_path,
        &owner,
        Some(json!({"suppress": false})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "promote a current stage audience member"
    );

    let (status, _) = call(
        &app,
        Method::PATCH,
        &member_path,
        &owner,
        Some(json!({"nick": "renamed"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a plain edit leaves voice alone");

    let (status, log) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/audit-logs"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let codes = audit_codes(&log);
    assert_eq!(codes.iter().filter(|c| **c == 26).count(), 1, "MEMBER_MOVE");
    assert_eq!(
        codes.iter().filter(|c| **c == 27).count(),
        1,
        "MEMBER_DISCONNECT"
    );

    let (moves,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM outbox WHERE payload->>'type' = 'VOICE_MEMBER_MOVE'")
            .fetch_one(&database.pool)
            .await
            .expect("count");
    assert_eq!(
        moves, 3,
        "the gateway is told about the move, disconnect and stage promotion"
    );
    let (target,): (Value,) = sqlx::query_as(
        "SELECT payload->'data' FROM outbox WHERE payload->>'type' = 'VOICE_MEMBER_MOVE' ORDER BY id LIMIT 1",
    )
    .fetch_one(&database.pool)
    .await
    .expect("event");
    assert_eq!(target["channel_id"], voice);
    assert_eq!(target["user_id"], guest_id);
    let (promotion,): (Value,) = sqlx::query_as(
        "SELECT payload->'data' FROM outbox WHERE payload->>'type' = 'VOICE_MEMBER_MOVE' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&database.pool)
    .await
    .expect("promotion event");
    assert_eq!(promotion["suppress"], false);
    assert!(promotion.get("channel_id").is_none());
}
