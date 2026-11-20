//! Token-independent recipient discovery: real HTTP, permissions, settings and PostgreSQL.

mod support;

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database, Database};

fn app(database: &Database) -> axum::Router {
    app_with_presence(database, None)
}

fn app_with_presence(
    database: &Database,
    presence: Option<Arc<dyn application::ports::PresenceDirectory>>,
) -> axum::Router {
    let (private, public) = jwt_keys();
    let mut ports = ports(&database.pool, &public, &private);
    if let Some(presence) = presence {
        ports.presence = presence;
    }
    interface::router(AppState {
        ports,
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    })
}

async fn account(app: &axum::Router, name: &str) -> (String, String) {
    let (status, body) = send(app, request(Method::POST, "/api/v1/auth/register", None,
        Some(json!({"email":format!("{name}@example.invalid"), "username":name, "password":"notification-policy-test-password"})))).await;
    assert_eq!(status, StatusCode::CREATED);
    (
        body["access_token"].as_str().unwrap().to_owned(),
        body["user"]["id"].as_str().unwrap().to_owned(),
    )
}

async fn ok_call(
    app: &axum::Router,
    method: Method,
    path: &str,
    access: &str,
    body: Option<Value>,
) -> Value {
    let (status, result) = call(app, method, path, access, body).await;
    assert!(status.is_success(), "{path}: {status} {result}");
    result
}

async fn dm(app: &axum::Router, access: &str, other: &str) -> String {
    ok_call(
        app,
        Method::POST,
        "/api/v1/users/@me/channels",
        access,
        Some(json!({"recipient_ids":[other]})),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn raw_page(
    app: &axum::Router,
    channel: &str,
    query: &str,
    authorized: bool,
) -> (StatusCode, Value) {
    let mut req = axum::http::Request::builder().method("GET").uri(format!(
        "/internal/v1/channels/{channel}/notification-recipients?{query}"
    ));
    if authorized {
        req = req.header("x-internal-token", INTERNAL_TOKEN);
    }
    send(app, req.body(axum::body::Body::empty()).unwrap()).await
}

async fn page(app: &axum::Router, channel: &str, author: &str, query: &str) -> Value {
    let (status, result) =
        raw_page(app, channel, &format!("author_id={author}{query}"), true).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    result
}

async fn guild(app: &axum::Router, owner: &str, members: &[&str]) -> (String, String) {
    let id = ok_call(
        app,
        Method::POST,
        "/api/v1/guilds",
        owner,
        Some(json!({"name":"Recipient policy"})),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let channels = ok_call(
        app,
        Method::GET,
        &format!("/api/v1/guilds/{id}/channels"),
        owner,
        None,
    )
    .await;
    let channel = channels
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["type"] == 0)
        .expect("guild has a text channel")["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let invite = ok_call(
        app,
        Method::POST,
        &format!("/api/v1/channels/{channel}/invites"),
        owner,
        Some(json!({})),
    )
    .await;
    let code = invite["code"].as_str().unwrap();
    for member in members {
        ok_call(
            app,
            Method::POST,
            &format!("/api/v1/invites/{code}"),
            member,
            None,
        )
        .await;
    }
    (id, channel)
}

#[tokio::test]
async fn a_desktop_only_dm_recipient_has_current_preferences_and_expired_mutes() {
    let database = start_database().await;
    let app = app(&database);
    let (alice, alice_id) = account(&app, "alice").await;
    let (bob, bob_id) = account(&app, "bob").await;
    let channel = dm(&app, &alice, &bob_id).await;
    let found = page(&app, &channel, &alice_id, "").await;
    assert_eq!(
        found["recipients"],
        json!([{"user_id":bob_id, "role_ids":[], "online":false,
        "level":0, "muted":false, "suppress_everyone":false, "suppress_roles":false}])
    );
    assert!(found["guild_id"].is_null() && found["next_after"].is_null());
    assert_eq!(found["channel_id"], channel);
    assert!(chrono::DateTime::parse_from_rfc3339(found["evaluated_at"].as_str().unwrap()).is_ok());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM push_tokens")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "directory membership must not require a mobile token"
    );

    ok_call(&app, Method::PATCH, "/api/v1/users/@me/guilds/@me/settings", &bob,
        Some(json!({"message_notifications":2, "muted":true, "suppress_everyone":true, "suppress_roles":true}))).await;
    let muted = page(&app, &channel, &alice_id, "").await;
    assert_eq!(muted["recipients"][0]["level"], 2);
    assert_eq!(muted["recipients"][0]["muted"], true);
    assert_eq!(muted["recipients"][0]["suppress_everyone"], true);
    assert_eq!(muted["recipients"][0]["suppress_roles"], true);
    let expired = (chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339();
    ok_call(
        &app,
        Method::PATCH,
        "/api/v1/users/@me/guilds/@me/settings",
        &bob,
        Some(json!({"muted":true, "mute_config":{"end_time":expired}})),
    )
    .await;
    assert_eq!(
        page(&app, &channel, &alice_id, "").await["recipients"][0]["muted"],
        false
    );
}

#[tokio::test]
async fn private_delivery_modes_return_no_audience_or_continuation() {
    let database = start_database().await;
    let app = app(&database);
    let (alice, alice_id) = account(&app, "alice").await;
    let (_bob, bob_id) = account(&app, "bob").await;
    let channel = dm(&app, &alice, &bob_id).await;
    for mode in ["ephemeral", "p2p"] {
        ok_call(
            &app,
            Method::PATCH,
            "/api/v1/users/@me/settings",
            &alice,
            Some(json!({"default_delivery_mode":mode})),
        )
        .await;
        let found = page(&app, &channel, &alice_id, "&limit=1").await;
        assert_eq!(found["delivery_mode"], mode);
        assert_eq!(found["recipients"], json!([]));
        assert!(found["next_after"].is_null());
    }
}

#[tokio::test]
async fn filtered_guild_pages_advance_past_the_author_and_hidden_members() {
    let database = start_database().await;
    let app = app(&database);
    let (owner, owner_id) = account(&app, "owner").await;
    let (hidden, hidden_id) = account(&app, "hidden").await;
    let (visible, visible_id) = account(&app, "visible").await;
    let (_outsider, _) = account(&app, "outsider").await;
    let (_, channel) = guild(&app, &owner, &[&hidden, &visible]).await;
    ok_call(
        &app,
        Method::PUT,
        &format!("/api/v1/channels/{channel}/permissions/{hidden_id}"),
        &owner,
        Some(json!({"type":1, "allow":"0", "deny":"1024"})),
    )
    .await;
    let first = page(&app, &channel, &owner_id, "&limit=1").await;
    assert_eq!(first["recipients"], json!([]));
    assert_eq!(first["next_after"], owner_id);
    let second = page(
        &app,
        &channel,
        &owner_id,
        &format!("&limit=1&after={owner_id}"),
    )
    .await;
    assert_eq!(second["recipients"], json!([]));
    assert_eq!(second["next_after"], hidden_id);
    let last = page(
        &app,
        &channel,
        &owner_id,
        &format!("&limit=1&after={hidden_id}"),
    )
    .await;
    assert_eq!(last["recipients"].as_array().unwrap().len(), 1);
    assert_eq!(last["recipients"][0]["user_id"], visible_id);
    assert!(last["next_after"].is_null());
}

#[tokio::test]
async fn guild_roles_parent_and_channel_preferences_match_push_policy_without_exposing_tokens() {
    let database = start_database().await;
    let app = app(&database);
    let (owner, owner_id) = account(&app, "owner").await;
    let (member, member_id) = account(&app, "member").await;
    let (guild_id, _) = guild(&app, &owner, &[&member]).await;
    let role = ok_call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{guild_id}/roles"),
        &owner,
        Some(json!({"name":"Readers"})),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    ok_call(
        &app,
        Method::PUT,
        &format!("/api/v1/guilds/{guild_id}/members/{member_id}/roles/{role}"),
        &owner,
        None,
    )
    .await;
    let parent = ok_call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        Some(json!({"name":"Category", "type":4})),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let channel = ok_call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        Some(json!({"name":"child", "type":0, "parent_id":parent})),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let settings = format!("/api/v1/users/@me/guilds/{guild_id}/settings");
    ok_call(
        &app,
        Method::PATCH,
        &settings,
        &member,
        Some(json!({"message_notifications":0,
        "suppress_everyone":true, "suppress_roles":true,
        "channel_overrides":[{"channel_id":parent, "message_notifications":1, "muted":true}]})),
    )
    .await;
    let inherited = page(&app, &channel, &owner_id, "").await;
    assert_eq!(inherited["recipients"][0]["role_ids"], json!([role]));
    assert_eq!(inherited["recipients"][0]["level"], 1);
    assert_eq!(inherited["recipients"][0]["muted"], true);
    ok_call(
        &app,
        Method::PATCH,
        &settings,
        &member,
        Some(json!({
        "channel_overrides":[{"channel_id":channel, "message_notifications":2}]})),
    )
    .await;
    let explicit = page(&app, &channel, &owner_id, "").await;
    assert_eq!(explicit["recipients"][0]["level"], 2);
    assert_eq!(
        explicit["recipients"][0]["muted"], true,
        "parent mute still applies"
    );
    ok_call(
        &app,
        Method::POST,
        "/api/v1/users/@me/devices/push",
        &member,
        Some(json!({"platform":"fcm", "token":"notification-policy-fixture"})),
    )
    .await;
    let req = axum::http::Request::builder()
        .method("GET")
        .uri(format!(
            "/internal/v1/channels/{channel}/push-targets?author_id={owner_id}"
        ))
        .header("x-internal-token", INTERNAL_TOKEN)
        .body(axum::body::Body::empty())
        .unwrap();
    let (status, pushed) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    for key in [
        "user_id",
        "role_ids",
        "online",
        "level",
        "muted",
        "suppress_everyone",
        "suppress_roles",
    ] {
        assert_eq!(
            explicit["recipients"][0][key], pushed["targets"][0][key],
            "{key}"
        );
    }
    assert!(explicit["recipients"][0].get("tokens").is_none());
    assert!(explicit["recipients"][0].get("voip_tokens").is_none());
}

#[tokio::test]
async fn directory_requires_internal_auth_and_valid_bounds() {
    let database = start_database().await;
    let app = app(&database);
    let (alice, alice_id) = account(&app, "alice").await;
    let (_, bob_id) = account(&app, "bob").await;
    let channel = dm(&app, &alice, &bob_id).await;
    assert_eq!(
        raw_page(&app, &channel, &format!("author_id={alice_id}"), false)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    for query in [
        format!("author_id={alice_id}&limit=0"),
        format!("author_id={alice_id}&limit=201"),
        format!("author_id={alice_id}&after=nope"),
        format!("author_id={alice_id}&after=18446744073709551615"),
        format!("author_id={alice_id}&after=%2B1"),
        "author_id=nope".into(),
        "limit=1".into(),
    ] {
        assert_eq!(
            raw_page(&app, &channel, &query, true).await.0,
            StatusCode::BAD_REQUEST,
            "{query}"
        );
    }
    assert_eq!(
        raw_page(&app, "1", &format!("author_id={alice_id}"), true)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn thread_directory_inherits_parent_access_and_hides_private_threads_from_nonmembers() {
    let database = start_database().await;
    let app = app(&database);
    let (owner, owner_id) = account(&app, "owner").await;
    let (member, member_id) = account(&app, "member").await;
    let (_, channel) = guild(&app, &owner, &[&member]).await;
    for (kind, expected) in [(11, 1), (12, 0)] {
        let thread = ok_call(
            &app,
            Method::POST,
            &format!("/api/v1/channels/{channel}/threads"),
            &owner,
            Some(json!({"name":"Thread", "type":kind})),
        )
        .await["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            page(&app, &thread, &owner_id, "").await["recipients"]
                .as_array()
                .unwrap()
                .len(),
            expected
        );
        ok_call(
            &app,
            Method::PUT,
            &format!("/api/v1/channels/{channel}/permissions/{member_id}"),
            &owner,
            Some(json!({"type":1, "allow":"0", "deny":"1024"})),
        )
        .await;
        assert_eq!(
            page(&app, &thread, &owner_id, "").await["recipients"],
            json!([])
        );
        ok_call(
            &app,
            Method::DELETE,
            &format!("/api/v1/channels/{channel}/permissions/{member_id}"),
            &owner,
            None,
        )
        .await;
    }
}

struct UnavailablePresence;

#[tokio::test]
async fn private_thread_invitation_and_leave_change_http_access_and_notification_audience() {
    let database = start_database().await;
    let app = app(&database);
    let (owner, owner_id) = account(&app, "threadowner").await;
    let (member, member_id) = account(&app, "threadmember").await;
    let (_, parent) = guild(&app, &owner, &[&member]).await;
    let created = ok_call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{parent}/threads"),
        &owner,
        Some(json!({"name":"Private", "type":12})),
    )
    .await;
    let thread = created["id"].as_str().unwrap();
    assert_eq!(created["member_count"], 1);
    assert_eq!(created["is_thread_member"], true);
    let thread_path = format!("/api/v1/channels/{thread}");
    assert_eq!(
        call(&app, Method::GET, &thread_path, &member, None).await.0,
        StatusCode::FORBIDDEN
    );
    let list = ok_call(
        &app,
        Method::GET,
        &format!("/api/v1/channels/{parent}/threads"),
        &member,
        None,
    )
    .await;
    assert!(list.as_array().unwrap().is_empty());

    let membership = format!("/api/v1/channels/{thread}/thread-members/{member_id}");
    assert_eq!(
        call(&app, Method::PUT, &membership, &owner, None).await.0,
        StatusCode::NO_CONTENT
    );
    let visible = ok_call(&app, Method::GET, &thread_path, &member, None).await;
    assert_eq!(visible["member_count"], 2);
    assert_eq!(visible["is_thread_member"], true);
    let list = ok_call(
        &app,
        Method::GET,
        &format!("/api/v1/channels/{parent}/threads"),
        &member,
        None,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(
        page(&app, thread, &owner_id, "").await["recipients"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/v1/channels/{thread}/thread-members/@me"),
            &member,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, Method::GET, &thread_path, &member, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert!(page(&app, thread, &owner_id, "").await["recipients"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[async_trait::async_trait]
impl application::ports::PresenceDirectory for UnavailablePresence {
    async fn online_count(&self, _users: &[domain::Snowflake]) -> application::AppResult<u64> {
        Ok(0)
    }
    async fn live_ids(
        &self,
        _users: &[domain::Snowflake],
    ) -> application::AppResult<Vec<domain::Snowflake>> {
        Err(application::AppError::internal(
            "injected presence lookup failure",
        ))
    }
}

#[tokio::test]
async fn an_unavailable_dependency_does_not_fall_back_to_unfiltered_recipients() {
    let database = start_database().await;
    let app = app_with_presence(&database, Some(Arc::new(UnavailablePresence)));
    let (alice, alice_id) = account(&app, "alice").await;
    let (_, bob_id) = account(&app, "bob").await;
    let channel = dm(&app, &alice, &bob_id).await;
    let (status, body) = raw_page(&app, &channel, &format!("author_id={alice_id}"), true).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.get("recipients").is_none());
}

#[tokio::test]
async fn thread_lifecycle_uses_injected_time_and_publishes_transitions() {
    use application::ports::ThreadLifecycleStore;
    use chrono::Duration;
    use infrastructure::postgres::PgThreadLifecycle;

    let database = start_database().await;
    let app = app(&database);
    let (owner, _) = account(&app, "lifecycleowner").await;
    let (_, parent) = guild(&app, &owner, &[]).await;
    let created = ok_call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{parent}/threads"),
        &owner,
        Some(json!({"name":"Lifecycle", "type":11, "auto_archive_duration":60})),
    )
    .await;
    let thread = created["id"].as_str().unwrap();
    let id = domain::Snowflake::new(thread.parse().unwrap());
    let initial: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT last_activity_at FROM thread_activity WHERE thread_id = $1")
            .bind(id.as_i64())
            .fetch_one(&database.pool)
            .await
            .unwrap();
    let lifecycle = PgThreadLifecycle::new(database.pool.clone(), "nl-home".into());
    assert_eq!(
        lifecycle
            .archive_due(initial + Duration::minutes(59), 10)
            .await
            .unwrap(),
        0
    );
    sqlx::query("ALTER TABLE outbox ADD CONSTRAINT reject_thread_update CHECK (payload->>'type' <> 'THREAD_UPDATE')")
        .execute(&database.pool).await.unwrap();
    assert!(lifecycle
        .archive_due(initial + Duration::minutes(60), 10)
        .await
        .is_err());
    let archived: bool = sqlx::query_scalar(
        "SELECT (thread_metadata->>'archived')::boolean FROM channels WHERE id = $1",
    )
    .bind(id.as_i64())
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert!(!archived, "outbox failure rolls back the archive");
    sqlx::query("ALTER TABLE outbox DROP CONSTRAINT reject_thread_update")
        .execute(&database.pool)
        .await
        .unwrap();
    assert_eq!(
        lifecycle
            .archive_due(initial + Duration::minutes(60), 10)
            .await
            .unwrap(),
        1
    );
    let path = format!("/api/v1/channels/{thread}");
    assert_eq!(
        ok_call(&app, Method::GET, &path, &owner, None).await["thread_metadata"]["archived"],
        true
    );
    lifecycle
        .record_message(
            id,
            initial + Duration::minutes(59),
            initial + Duration::minutes(61),
        )
        .await
        .unwrap();
    assert_eq!(
        ok_call(&app, Method::GET, &path, &owner, None).await["thread_metadata"]["archived"],
        false
    );
    assert_eq!(
        lifecycle
            .archive_due(initial + Duration::minutes(119), 10)
            .await
            .unwrap(),
        1
    );
    lifecycle
        .record_message(
            id,
            initial + Duration::minutes(59),
            initial + Duration::minutes(120),
        )
        .await
        .unwrap();
    assert_eq!(
        ok_call(&app, Method::GET, &path, &owner, None).await["thread_metadata"]["archived"],
        true
    );
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE payload->>'type' = 'THREAD_UPDATE' AND payload->>'channel_id' = $1")
        .bind(thread).fetch_one(&database.pool).await.unwrap();
    assert_eq!(events, 3);
    ok_call(
        &app,
        Method::PATCH,
        &path,
        &owner,
        Some(json!({"locked":true})),
    )
    .await;
    lifecycle
        .record_message(
            id,
            initial + Duration::minutes(120),
            initial + Duration::minutes(120),
        )
        .await
        .unwrap();
    assert_eq!(
        ok_call(&app, Method::GET, &path, &owner, None).await["thread_metadata"]["archived"],
        true
    );
}
