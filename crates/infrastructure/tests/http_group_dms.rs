//! HTTP contract tests for group DM management (api#29): add and remove recipients, the member
//! cap, owner-only removal, leaving with ownership transfer, rename, system messages and events.

mod support;

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send};
use support::{jwt_keys, ports_with_messages, start_database, Database, RecordingMessages};

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

async fn events(database: &Database, kind: &str) -> Vec<Value> {
    let rows: Vec<(Value,)> =
        sqlx::query_as("SELECT payload FROM outbox WHERE payload->>'type' = $1 ORDER BY id")
            .bind(kind)
            .fetch_all(&database.pool)
            .await
            .expect("outbox query");
    rows.into_iter().map(|(payload,)| payload).collect()
}

fn message_types(messages: &RecordingMessages) -> Vec<(u64, Option<String>)> {
    messages
        .bodies
        .lock()
        .unwrap()
        .iter()
        .map(|b| {
            (
                b["type"].as_u64().unwrap(),
                b["target_user_id"].as_str().map(str::to_owned),
            )
        })
        .collect()
}

#[tokio::test]
async fn group_dm_membership_lifecycle() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let messages = Arc::new(RecordingMessages::default());
    let app = interface::router(AppState {
        ports: ports_with_messages(&database.pool, &public, &private, messages.clone()),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let (owner, owner_id) = register(&app, "owner").await;
    let (bob, bob_id) = register(&app, "bob").await;
    let (_carol, carol_id) = register(&app, "carol").await;
    let (dave, dave_id) = register(&app, "dave").await;
    let (outsider, _) = register(&app, "outsider").await;

    let (status, group) = call(
        &app,
        Method::POST,
        "/api/v1/users/@me/channels",
        &owner,
        Some(json!({"recipient_ids": [bob_id, carol_id]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(group["type"], 3);
    let channel_id = group["id"].as_str().unwrap().to_owned();
    let path = |user: &str| format!("/api/v1/channels/{channel_id}/recipients/{user}");

    // A non-member cannot add; a member can, and adding twice is a no-op.
    let (status, _) = call(&app, Method::PUT, &path(&dave_id), &outsider, None).await;
    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "{status}"
    );
    let (status, _) = call(&app, Method::PUT, &path(&dave_id), &bob, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, Method::PUT, &path(&dave_id), &bob, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, Method::PUT, &path("999999999"), &bob, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (_, listed) = call(&app, Method::GET, "/api/v1/users/@me/channels", &dave, None).await;
    assert_eq!(
        listed.as_array().map(Vec::len),
        Some(1),
        "the new member sees the group"
    );
    assert_eq!(listed[0]["recipients"].as_array().map(Vec::len), Some(3));

    let added = events(&database, "CHANNEL_RECIPIENT_ADD").await;
    assert_eq!(added.len(), 1, "no event for the idempotent repeat");
    assert_eq!(added[0]["data"]["user"]["id"], json!(dave_id));
    let mut notified: Vec<String> =
        serde_json::from_value(added[0]["target_user_ids"].clone()).unwrap();
    notified.sort();
    let mut expected = vec![owner_id.clone(), bob_id.clone(), carol_id.clone()];
    expected.sort();
    assert_eq!(
        notified, expected,
        "the new member is not told about their own addition"
    );
    let created = events(&database, "CHANNEL_CREATE").await;
    assert!(created
        .iter()
        .any(|e| e["target_user_ids"] == json!([dave_id])));
    assert_eq!(message_types(&messages)[0], (1, Some(dave_id.clone())));

    // Only the owner removes others.
    let (status, _) = call(&app, Method::DELETE, &path(&carol_id), &bob, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&app, Method::DELETE, &path(&carol_id), &owner, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, Method::DELETE, &path(&carol_id), &owner, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "already gone");
    let removed = events(&database, "CHANNEL_RECIPIENT_REMOVE").await;
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0]["data"]["user"]["id"], json!(carol_id));
    let deleted = events(&database, "CHANNEL_DELETE").await;
    assert!(deleted
        .iter()
        .any(|e| e["target_user_ids"] == json!([carol_id])));
    assert_eq!(message_types(&messages)[1], (2, Some(carol_id.clone())));

    // Rename posts a system message and a CHANNEL_UPDATE.
    let (status, renamed) = call(
        &app,
        Method::PATCH,
        &format!("/api/v1/channels/{channel_id}"),
        &bob,
        Some(json!({"name": "friends"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(renamed["name"], "friends");
    assert_eq!(message_types(&messages)[2].0, 4);
    assert_eq!(messages.bodies.lock().unwrap()[2]["content"], "friends");
    assert!(!events(&database, "CHANNEL_UPDATE").await.is_empty());

    // The owner leaves; ownership moves to a remaining member.
    let (status, _) = call(&app, Method::DELETE, &path("@me"), &owner, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, after) = call(
        &app,
        Method::GET,
        &format!("/api/v1/channels/{channel_id}"),
        &bob,
        None,
    )
    .await;
    let new_owner = after["owner_id"].as_str().unwrap();
    assert_ne!(new_owner, owner_id);
    assert!([&bob_id, &dave_id].contains(&&new_owner.to_owned()));
    let (status, _) = call(
        &app,
        Method::GET,
        &format!("/api/v1/channels/{channel_id}"),
        &owner,
        None,
    )
    .await;
    assert_ne!(status, StatusCode::OK, "the owner no longer sees the group");
}

#[tokio::test]
async fn group_dm_is_capped_at_ten_members() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports_with_messages(
            &database.pool,
            &public,
            &private,
            Arc::new(RecordingMessages::default()),
        ),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let (owner, _) = register(&app, "owner").await;
    let mut others = Vec::new();
    for i in 0..10 {
        others.push(register(&app, &format!("user{i}")).await.1);
    }
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/users/@me/channels",
        &owner,
        Some(json!({"recipient_ids": others[..10]})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "owner plus ten is eleven members"
    );
    let (status, group) = call(
        &app,
        Method::POST,
        "/api/v1/users/@me/channels",
        &owner,
        Some(json!({"recipient_ids": others[..9]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "owner plus nine is ten members");
    let id = group["id"].as_str().unwrap();
    let (status, _) = call(
        &app,
        Method::PUT,
        &format!("/api/v1/channels/{id}/recipients/{}", others[9]),
        &owner,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "the eleventh member is refused"
    );
}
