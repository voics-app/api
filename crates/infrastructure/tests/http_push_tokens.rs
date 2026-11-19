//! HTTP contract tests for push tokens: registration and refresh per session, removal at logout,
//! session revocation and by the notifier, and the internal push-targets endpoint the notifier
//! reads.

mod support;

#[path = "http_push_tokens/age_restricted.rs"]
mod age_restricted;

use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use application::ports::PresenceDirectory;
use application::AppResult;
use domain::Snowflake;
use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database, Database};

const PASSWORD: &str = "a-long-enough-passphrase";
const TOKEN_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TOKEN_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/// Presence where the listed users hold a live gateway session.
struct LivePresence(Vec<Snowflake>);

#[async_trait]
impl PresenceDirectory for LivePresence {
    async fn online_count(&self, _user_ids: &[Snowflake]) -> AppResult<u64> {
        Ok(0)
    }

    async fn live_ids(&self, user_ids: &[Snowflake]) -> AppResult<Vec<Snowflake>> {
        Ok(user_ids
            .iter()
            .copied()
            .filter(|id| self.0.contains(id))
            .collect())
    }
}

fn app_with_live(database: &Database, live: Vec<Snowflake>) -> axum::Router {
    let (private, public) = jwt_keys();
    let mut ports = ports(&database.pool, &public, &private);
    ports.presence = Arc::new(LivePresence(live));
    interface::router(AppState {
        ports,
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    })
}

fn app(database: &Database) -> axum::Router {
    app_with_live(database, Vec::new())
}

async fn register(app: &axum::Router, username: &str) -> (String, String) {
    let (status, body) = send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({
                "email": format!("{username}@example.com"),
                "username": username,
                "password": PASSWORD,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "register {username}");
    (
        body["access_token"].as_str().expect("token").to_owned(),
        body["user"]["id"].as_str().expect("id").to_owned(),
    )
}

async fn login(app: &axum::Router, username: &str) -> String {
    let (status, body) = send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/login",
            None,
            Some(json!({ "email": format!("{username}@example.com"), "password": PASSWORD })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "login {username}");
    body["access_token"].as_str().expect("token").to_owned()
}

async fn put_token(app: &axum::Router, access: &str, token: &str) -> StatusCode {
    call(
        app,
        Method::POST,
        "/api/v1/users/@me/devices/push",
        access,
        Some(json!({"platform": "apns", "token": token, "environment": "sandbox"})),
    )
    .await
    .0
}

async fn stored(database: &Database) -> Vec<(i64, String, String, bool)> {
    sqlx::query_as(
        "SELECT user_id, token, environment, previews FROM push_tokens ORDER BY created_at, token",
    )
    .fetch_all(&database.pool)
    .await
    .expect("push_tokens query")
}

fn internal(
    method: Method,
    path: &str,
    body: Option<Value>,
) -> axum::http::Request<axum::body::Body> {
    let builder = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("x-internal-token", INTERNAL_TOKEN);
    match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .expect("request"),
        None => builder.body(axum::body::Body::empty()).expect("request"),
    }
}

async fn targets(app: &axum::Router, channel_id: &str, author_id: &str) -> Value {
    let (status, body) = send(
        app,
        internal(
            Method::GET,
            &format!("/internal/v1/channels/{channel_id}/push-targets?author_id={author_id}"),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "push-targets: {body}");
    body
}

async fn dm(app: &axum::Router, from: &str, to_id: &str) -> String {
    let (status, channel) = call(
        app,
        Method::POST,
        "/api/v1/users/@me/channels",
        from,
        Some(json!({"recipient_ids": [to_id]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    channel["id"].as_str().expect("channel id").to_owned()
}

#[tokio::test]
async fn a_session_keeps_one_token_that_a_refresh_replaces_and_a_delete_removes() {
    let database = start_database().await;
    let app = app(&database);
    let (alice, alice_id) = register(&app, "alice").await;

    assert_eq!(
        put_token(&app, &alice, TOKEN_A).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        put_token(&app, &alice, TOKEN_A).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        stored(&database).await.len(),
        1,
        "same token twice is one row"
    );

    assert_eq!(
        put_token(&app, &alice, TOKEN_B).await,
        StatusCode::NO_CONTENT
    );
    let rows = stored(&database).await;
    assert_eq!(rows.len(), 1, "a rotated token replaces the old one");
    assert_eq!(
        rows[0],
        (
            alice_id.parse().expect("id"),
            TOKEN_B.into(),
            "sandbox".into(),
            true
        )
    );

    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/users/@me/devices/push",
        &alice,
        Some(
            json!({"platform": "apns", "token": TOKEN_B, "previews": false, "only_mentions": true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let flags: (bool, bool, String) =
        sqlx::query_as("SELECT previews, only_mentions, environment FROM push_tokens")
            .fetch_one(&database.pool)
            .await
            .expect("flags");
    assert_eq!(
        flags,
        (false, true, "production".into()),
        "the device flags and the default environment are stored"
    );

    let (status, _) = call(
        &app,
        Method::DELETE,
        "/api/v1/users/@me/devices/push",
        &alice,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(stored(&database).await.is_empty());
    let (status, _) = call(
        &app,
        Method::DELETE,
        "/api/v1/users/@me/devices/push",
        &alice,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "removing twice is fine");
}

#[tokio::test]
async fn registration_validates_the_request_and_needs_a_user_session() {
    let database = start_database().await;
    let app = app(&database);
    let (alice, _) = register(&app, "alice").await;

    for body in [
        json!({"platform": "wns", "token": TOKEN_A}),
        json!({"platform": "apns", "token": "not hex!"}),
        json!({"platform": "apns", "token": ""}),
        json!({"platform": "apns", "token": TOKEN_A, "environment": "staging"}),
    ] {
        let (status, _) = call(
            &app,
            Method::POST,
            "/api/v1/users/@me/devices/push",
            &alice,
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
    let (status, _) = send(
        &app,
        request(
            Method::POST,
            "/api/v1/users/@me/devices/push",
            None,
            Some(json!({"platform": "apns", "token": TOKEN_A})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(stored(&database).await.is_empty());
}

#[tokio::test]
async fn the_same_installation_moving_to_another_account_takes_its_token_along() {
    let database = start_database().await;
    let app = app(&database);
    let (alice, _) = register(&app, "alice").await;
    let (bob, bob_id) = register(&app, "bob").await;

    assert_eq!(
        put_token(&app, &alice, TOKEN_A).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(put_token(&app, &bob, TOKEN_A).await, StatusCode::NO_CONTENT);

    let rows = stored(&database).await;
    assert_eq!(rows.len(), 1, "a provider token belongs to one session");
    assert_eq!(rows[0].0, bob_id.parse::<i64>().expect("id"));
}

#[tokio::test]
async fn logout_and_remote_revocation_remove_the_token_of_that_session_only() {
    let database = start_database().await;
    let app = app(&database);
    let (phone, _) = register(&app, "alice").await;
    let tablet = login(&app, "alice").await;
    let laptop = login(&app, "alice").await;
    assert_eq!(
        put_token(&app, &phone, TOKEN_A).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        put_token(&app, &tablet, TOKEN_B).await,
        StatusCode::NO_CONTENT
    );

    let (status, _) = call(&app, Method::POST, "/api/v1/auth/logout", &phone, None).await;
    assert!(status.is_success(), "logout: {status}");
    let rows = stored(&database).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, TOKEN_B);

    let (_, sessions) = call(
        &app,
        Method::GET,
        "/api/v1/users/@me/sessions",
        &laptop,
        None,
    )
    .await;
    let tablet_session: String = sqlx::query_scalar("SELECT session_id::text FROM push_tokens")
        .fetch_one(&database.pool)
        .await
        .expect("tablet session");
    assert!(sessions
        .as_array()
        .expect("sessions")
        .iter()
        .any(|s| s["id"] == tablet_session.as_str()));
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/v1/users/@me/sessions/{tablet_session}"),
        &laptop,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        stored(&database).await.is_empty(),
        "revoking drops the token"
    );
}

#[tokio::test]
async fn logout_everywhere_drops_the_tokens_of_the_other_sessions() {
    let database = start_database().await;
    let app = app(&database);
    let (phone, _) = register(&app, "alice").await;
    let laptop = login(&app, "alice").await;
    assert_eq!(
        put_token(&app, &phone, TOKEN_A).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        put_token(&app, &laptop, TOKEN_B).await,
        StatusCode::NO_CONTENT
    );

    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/users/@me/sessions/logout-all",
        &laptop,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let rows = stored(&database).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, TOKEN_B, "the current session keeps its token");
}

#[tokio::test]
async fn a_user_keeps_at_most_twenty_tokens_and_the_oldest_refresh_is_dropped() {
    let database = start_database().await;
    let app = app(&database);
    let (first, _) = register(&app, "alice").await;
    assert_eq!(
        put_token(&app, &first, TOKEN_A).await,
        StatusCode::NO_CONTENT
    );
    for n in 0..20 {
        let access = login(&app, "alice").await;
        let token = format!("{n:064x}");
        assert_eq!(
            put_token(&app, &access, &token).await,
            StatusCode::NO_CONTENT
        );
    }
    let rows = stored(&database).await;
    assert_eq!(rows.len(), 20);
    assert!(
        rows.iter().all(|row| row.1 != TOKEN_A),
        "the first token was trimmed"
    );
}

#[tokio::test]
async fn the_notifier_removes_a_rejected_token_through_the_internal_api() {
    let database = start_database().await;
    let app = app(&database);
    let (alice, _) = register(&app, "alice").await;
    assert_eq!(
        put_token(&app, &alice, TOKEN_A).await,
        StatusCode::NO_CONTENT
    );

    let (status, _) = send(
        &app,
        request(
            Method::DELETE,
            "/internal/v1/push-tokens",
            None,
            Some(json!({"platform": "apns", "token": TOKEN_A})),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the internal token is required"
    );

    let (status, _) = send(
        &app,
        internal(
            Method::DELETE,
            "/internal/v1/push-tokens",
            Some(json!({"platform": "apns", "token": TOKEN_A})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(stored(&database).await.is_empty());
}

#[tokio::test]
async fn dm_push_targets_list_the_recipient_with_tokens_settings_and_presence() {
    let database = start_database().await;
    let (_, bob_id) = {
        let probe = app(&database);
        register(&probe, "bob").await
    };
    let app = app_with_live(&database, vec![bob_id.parse().expect("id")]);
    let (alice, alice_id) = register(&app, "alice").await;
    let bob = login(&app, "bob").await;
    let channel = dm(&app, &alice, &bob_id).await;
    assert_eq!(put_token(&app, &bob, TOKEN_B).await, StatusCode::NO_CONTENT);
    assert_eq!(
        put_token(&app, &alice, TOKEN_A).await,
        StatusCode::NO_CONTENT
    );

    let found = targets(&app, &channel, &alice_id).await;

    assert_eq!(found["delivery_mode"], "standard");
    let list = found["targets"].as_array().expect("targets");
    assert_eq!(list.len(), 1, "the author is not a target");
    assert_eq!(list[0]["user_id"], bob_id.as_str());
    assert_eq!(list[0]["online"], true);
    assert_eq!(list[0]["level"], 0);
    assert_eq!(list[0]["muted"], false);
    assert_eq!(
        list[0]["tokens"],
        json!([{"platform": "apns", "token": TOKEN_B, "environment": "sandbox", "previews": true, "only_mentions": false}])
    );

    // A DM mute and level 2 show up in the same call.
    let (status, _) = call(
        &app,
        Method::PATCH,
        "/api/v1/users/@me/guilds/@me/settings",
        &bob,
        Some(json!({"message_notifications": 1, "muted": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let list = targets(&app, &channel, &alice_id).await["targets"].clone();
    assert_eq!(list[0]["level"], 1);
    assert_eq!(list[0]["muted"], true);
}

#[tokio::test]
async fn guild_push_targets_only_list_members_who_can_view_the_channel() {
    let database = start_database().await;
    let app = app(&database);
    let (owner, owner_id) = register(&app, "owner").await;
    let (member, member_id) = register(&app, "member").await;
    let (outsider, _) = register(&app, "outsider").await;

    let (_, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name": "Voics HQ"})),
    )
    .await;
    let guild_id = guild["id"].as_str().expect("guild").to_owned();
    let (_, channels) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        None,
    )
    .await;
    let channel_id = channels[0]["id"].as_str().expect("channel").to_owned();
    let (_, invite) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{channel_id}/invites"),
        &owner,
        Some(json!({})),
    )
    .await;
    let code = invite["code"].as_str().expect("code").to_owned();
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/v1/invites/{code}"),
        &member,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        put_token(&app, &owner, TOKEN_A).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        put_token(&app, &member, TOKEN_B).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        put_token(&app, &outsider, &"c".repeat(64)).await,
        StatusCode::NO_CONTENT
    );

    let found = targets(&app, &channel_id, &owner_id).await;

    assert_eq!(found["guild_name"], "Voics HQ");
    assert_eq!(found["guild_id"], guild_id.as_str());
    assert!(found["channel_name"].is_string());
    let list = found["targets"].as_array().expect("targets");
    assert_eq!(list.len(), 1, "no author, no outsider");
    assert_eq!(list[0]["user_id"], member_id.as_str());
    assert_eq!(list[0]["online"], false);
}

#[tokio::test]
async fn pruning_stale_devices_also_drops_tokens_nobody_refreshed() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let ports = ports(&database.pool, &public, &private);
    let app = app(&database);
    let (alice, _) = register(&app, "alice").await;
    let laptop = login(&app, "alice").await;
    assert_eq!(
        put_token(&app, &alice, TOKEN_A).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        put_token(&app, &laptop, TOKEN_B).await,
        StatusCode::NO_CONTENT
    );
    sqlx::query("UPDATE push_tokens SET updated_at = now() - interval '90 days' WHERE token = $1")
        .bind(TOKEN_A)
        .execute(&database.pool)
        .await
        .expect("age the token");

    application::use_cases::devices::prune_stale::execute(&ports)
        .await
        .expect("prune");

    let rows = stored(&database).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, TOKEN_B);
}

#[tokio::test]
async fn manual_push_lists_one_users_live_tokens_and_needs_the_internal_token() {
    let database = start_database().await;
    let app = app(&database);
    let (alice, alice_id) = register(&app, "alice").await;
    let (_, nobody_id) = register(&app, "nobody").await;
    let laptop = login(&app, "alice").await;
    assert_eq!(
        put_token(&app, &alice, TOKEN_A).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        put_token(&app, &laptop, TOKEN_B).await,
        StatusCode::NO_CONTENT
    );

    let (status, body) = send(
        &app,
        internal(
            Method::GET,
            &format!("/internal/v1/users/{alice_id}/push-tokens"),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["users"].as_array().expect("users").len(), 1);
    assert_eq!(body["users"][0]["user_id"], alice_id.as_str());
    assert_eq!(
        body["users"][0]["tokens"].as_array().expect("tokens").len(),
        2
    );
    assert_eq!(body["next_after"], Value::Null);

    // A revoked session's token is not live any more.
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = (SELECT session_id FROM push_tokens WHERE token = $1)")
        .bind(TOKEN_A)
        .execute(&database.pool)
        .await
        .expect("revoke");
    let (_, body) = send(
        &app,
        internal(
            Method::GET,
            &format!("/internal/v1/users/{alice_id}/push-tokens"),
            None,
        ),
    )
    .await;
    assert_eq!(
        body["users"][0]["tokens"].as_array().expect("tokens").len(),
        1
    );

    // A user without a token gives an empty list, not an error.
    let (status, body) = send(
        &app,
        internal(
            Method::GET,
            &format!("/internal/v1/users/{nobody_id}/push-tokens"),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["users"], json!([]));

    let (status, _) = send(
        &app,
        request(
            Method::GET,
            &format!("/internal/v1/users/{alice_id}/push-tokens"),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn manual_push_pages_through_every_user_with_a_live_token() {
    let database = start_database().await;
    let app = app(&database);
    let (alice, alice_id) = register(&app, "alice").await;
    let (bob, bob_id) = register(&app, "bob").await;
    let (_, _no_token) = register(&app, "carol").await;
    let (dave, dave_id) = register(&app, "dave").await;
    assert_eq!(
        put_token(&app, &alice, TOKEN_A).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(put_token(&app, &bob, TOKEN_B).await, StatusCode::NO_CONTENT);
    assert_eq!(
        put_token(&app, &dave, &"d".repeat(64)).await,
        StatusCode::NO_CONTENT
    );

    let mut seen = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..5 {
        let path = match &after {
            Some(id) => format!("/internal/v1/push-tokens?limit=2&after={id}"),
            None => "/internal/v1/push-tokens?limit=2".to_owned(),
        };
        let (status, body) = send(&app, internal(Method::GET, &path, None)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for user in body["users"].as_array().expect("users") {
            seen.push(user["user_id"].as_str().expect("id").to_owned());
        }
        match body["next_after"].as_str() {
            Some(next) => after = Some(next.to_owned()),
            None => break,
        }
    }

    let mut expected = vec![alice_id, bob_id, dave_id];
    expected.sort_by_key(|id| id.parse::<i64>().expect("snowflake"));
    assert_eq!(
        seen, expected,
        "each user with a token once, in id order, none without"
    );
}

#[tokio::test]
async fn badge_visibility_is_private_and_cannot_grant_badges() {
    let database = start_database().await;
    let app = app(&database);
    let (alice, alice_id) = register(&app, "badgealice").await;
    let (bob, _) = register(&app, "badgebob").await;
    sqlx::query("INSERT INTO user_badges (user_id, badge_id) VALUES ($1, 'fixture_one'), ($1, 'fixture_two')")
        .bind(alice_id.parse::<i64>().unwrap()).execute(&database.pool).await.unwrap();
    let (status, _) = call(
        &app,
        Method::PUT,
        "/api/v1/users/@me/badges/fixture_one",
        &alice,
        Some(json!({"visible": false})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, own) = call(&app, Method::GET, "/api/v1/users/@me/badges", &alice, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        own,
        json!([{"id":"fixture_one","visible":false},{"id":"fixture_two","visible":true}])
    );
    let (status, public) = call(
        &app,
        Method::GET,
        &format!("/api/v1/users/{alice_id}/badges"),
        &bob,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(public, json!(["fixture_two"]));
    let (status, _) = call(
        &app,
        Method::PUT,
        "/api/v1/users/@me/badges/fixture_one",
        &bob,
        Some(json!({"visible": true})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, public) = call(
        &app,
        Method::GET,
        &format!("/api/v1/users/{alice_id}/badges"),
        &bob,
        None,
    )
    .await;
    assert_eq!(public, json!(["fixture_two"]));
}
