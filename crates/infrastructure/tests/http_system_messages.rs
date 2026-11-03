//! HTTP contract tests for the guild system messages api posts through the messages service
//! (api#94): member join (type 7) and thread created (type 18), plus the editable system channel
//! settings and guild overview fields (api#37) that gate them.

mod support;

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send};
use support::{jwt_keys, ports_with_messages, start_database, RecordingMessages};

async fn register(app: &axum::Router, name: &str) -> (String, String) {
    let (status, body) = send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({
                "email": format!("{name}@example.com"),
                "username": name,
                "password": "a-long-enough-passphrase"
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "register {name}");
    (
        body["access_token"].as_str().expect("token").to_owned(),
        body["user"]["id"].as_str().expect("id").to_owned(),
    )
}

async fn invite_code(app: &axum::Router, owner: &str, channel_id: &str) -> String {
    let (status, invite) = call(
        app,
        Method::POST,
        &format!("/api/v1/channels/{channel_id}/invites"),
        owner,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    invite["code"].as_str().expect("code").to_owned()
}

async fn join(app: &axum::Router, token: &str, code: &str) {
    let (status, _) = call(
        app,
        Method::POST,
        &format!("/api/v1/invites/{code}"),
        token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

async fn channel_of_kind(app: &axum::Router, owner: &str, guild_id: &str, kind: u64) -> String {
    let (_, channels) = call(
        app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        owner,
        None,
    )
    .await;
    channels
        .as_array()
        .expect("channels")
        .iter()
        .find(|c| c["type"] == kind)
        .expect("channel of kind")["id"]
        .as_str()
        .expect("id")
        .to_owned()
}

fn recorded(messages: &RecordingMessages) -> Vec<(String, Value)> {
    let channels = messages.channels.lock().unwrap();
    let bodies = messages.bodies.lock().unwrap();
    channels
        .iter()
        .map(|c| c.to_string())
        .zip(bodies.iter().cloned())
        .collect()
}

fn router(database: &support::Database, messages: Option<Arc<RecordingMessages>>) -> axum::Router {
    let (private, public) = jwt_keys();
    let ports = match messages {
        Some(messages) => ports_with_messages(&database.pool, &public, &private, messages),
        None => support::ports(&database.pool, &public, &private),
    };
    interface::router(AppState {
        ports,
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    })
}

async fn create_guild(app: &axum::Router, owner: &str) -> Value {
    let (status, guild) = call(
        app,
        Method::POST,
        "/api/v1/guilds",
        owner,
        Some(json!({"name": "HQ"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    guild
}

#[tokio::test]
async fn joins_and_threads_post_system_messages_unless_suppressed() {
    let database = start_database().await;
    let messages = Arc::new(RecordingMessages::default());
    let app = router(&database, Some(messages.clone()));
    let (owner, _) = register(&app, "owner").await;
    let (ann, ann_id) = register(&app, "ann").await;
    let (ben, _) = register(&app, "ben").await;
    let (cat, _) = register(&app, "cat").await;

    let guild = create_guild(&app, &owner).await;
    let guild_id = guild["id"].as_str().expect("guild id").to_owned();
    let text = channel_of_kind(&app, &owner, &guild_id, 0).await;
    assert_eq!(
        guild["system_channel_id"], text,
        "new guilds announce in the general channel"
    );
    assert!(
        recorded(&messages).is_empty(),
        "creating a guild posts nothing"
    );

    let code = invite_code(&app, &owner, &text).await;
    join(&app, &ann, &code).await;
    let posted = recorded(&messages);
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].0, text);
    assert_eq!(posted[0].1["type"], 7);
    assert_eq!(posted[0].1["author_id"], ann_id);

    let (status, thread) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{text}/threads"),
        &owner,
        Some(json!({"name": "plans"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{thread}");
    let posted = recorded(&messages);
    assert_eq!(posted.len(), 2);
    assert_eq!(posted[1].0, text, "announced in the parent channel");
    assert_eq!(posted[1].1["type"], 18);
    assert_eq!(posted[1].1["thread_id"], thread["id"]);
    assert_eq!(posted[1].1["content"], "plans");

    let path = format!("/api/v1/guilds/{guild_id}");
    let (status, patched) = call(
        &app,
        Method::PATCH,
        &path,
        &owner,
        Some(json!({"system_channel_flags": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(patched["system_channel_flags"], 1);
    join(&app, &ben, &code).await;
    assert_eq!(
        recorded(&messages).len(),
        2,
        "suppressed joins post nothing"
    );

    let (status, _) = call(
        &app,
        Method::PATCH,
        &path,
        &owner,
        Some(json!({"system_channel_flags": 0, "system_channel_id": null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    join(&app, &cat, &code).await;
    assert_eq!(
        recorded(&messages).len(),
        2,
        "no system channel, no join message"
    );
}

#[tokio::test]
async fn a_failing_messages_service_never_fails_the_join_or_the_thread() {
    let database = start_database().await;
    let app = router(&database, None);
    let (owner, _) = register(&app, "owner").await;
    let (guest, _) = register(&app, "guest").await;
    let guild = create_guild(&app, &owner).await;
    let guild_id = guild["id"].as_str().expect("guild id").to_owned();
    let text = channel_of_kind(&app, &owner, &guild_id, 0).await;
    let code = invite_code(&app, &owner, &text).await;
    join(&app, &guest, &code).await;
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{text}/threads"),
        &owner,
        Some(json!({"name": "plans"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn guild_overview_fields_are_validated_and_audited() {
    let database = start_database().await;
    let app = router(&database, None);
    let (owner, _) = register(&app, "owner").await;
    let (guest, _) = register(&app, "guest").await;
    let guild = create_guild(&app, &owner).await;
    let guild_id = guild["id"].as_str().expect("guild id").to_owned();
    let path = format!("/api/v1/guilds/{guild_id}");
    let text = channel_of_kind(&app, &owner, &guild_id, 0).await;
    let voice = channel_of_kind(&app, &owner, &guild_id, 2).await;

    let patch = |body: Value| call(&app, Method::PATCH, &path, &owner, Some(body));
    for bad in [
        json!({"system_channel_flags": 64}),
        json!({"afk_timeout": 61}),
        json!({"afk_channel_id": text}),
        json!({"system_channel_id": voice}),
        json!({"default_message_notifications": 2}),
        json!({"banner": "not-a-data-uri"}),
    ] {
        let (status, body) = patch(bad.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }

    let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    let (status, updated) = patch(json!({
        "afk_channel_id": voice,
        "afk_timeout": 900,
        "default_message_notifications": 1,
        "system_channel_flags": 5,
        "banner": png,
        "splash": png,
    }))
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["afk_channel_id"], voice);
    assert_eq!(updated["afk_timeout"], 900);
    assert_eq!(updated["default_message_notifications"], 1);
    assert_eq!(updated["system_channel_flags"], 5);
    assert!(updated["banner"].as_str().is_some());
    assert!(updated["splash"].as_str().is_some());

    let (_, fetched) = call(&app, Method::GET, &path, &owner, None).await;
    assert_eq!(fetched["banner"], updated["banner"]);
    assert_eq!(fetched["system_channel_flags"], 5);

    let (status, cleared) = patch(json!({"banner": null, "afk_channel_id": null})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(cleared["banner"].is_null() && cleared["afk_channel_id"].is_null());

    let (status, _) = call(
        &app,
        Method::PATCH,
        &path,
        &guest,
        Some(json!({"afk_timeout": 60})),
    )
    .await;
    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "non-members cannot edit: {status}"
    );

    let (_, log) = call(
        &app,
        Method::GET,
        &format!("{path}/audit-logs"),
        &owner,
        None,
    )
    .await;
    let keys: Vec<&str> = log["audit_log_entries"]
        .as_array()
        .expect("entries")
        .iter()
        .flat_map(|e| e["changes"].as_array().into_iter().flatten())
        .filter_map(|c| c["key"].as_str())
        .collect();
    for key in [
        "afk_channel_id",
        "system_channel_flags",
        "banner_hash",
        "splash_hash",
    ] {
        assert!(keys.contains(&key), "audit log records {key}: {keys:?}");
    }
}
