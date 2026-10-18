//! HTTP contract tests for device registration and removal: each change
//! writes a USER_DEVICES_UPDATE to the outbox, targeted at the user and every
//! user who shares a DM, group DM or guild with them.

mod support;

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database, Database};

const IDENTITY_KEY_B64: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

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
                "password": "a-long-enough-passphrase",
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

/// Every USER_DEVICES_UPDATE payload in the outbox, oldest first.
async fn device_updates(database: &Database) -> Vec<Value> {
    let rows: Vec<(Value,)> = sqlx::query_as(
        "SELECT payload FROM outbox WHERE payload->>'type' = 'USER_DEVICES_UPDATE' ORDER BY id",
    )
    .fetch_all(&database.pool)
    .await
    .expect("outbox query");
    rows.into_iter().map(|(payload,)| payload).collect()
}

fn target_set(payload: &Value) -> BTreeSet<String> {
    payload["target_user_ids"]
        .as_array()
        .expect("targets are an array")
        .iter()
        .map(|id| id.as_str().expect("id").to_owned())
        .collect()
}

#[tokio::test]
async fn device_changes_publish_user_devices_update_to_everyone_sharing_a_context() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });

    let (alice, alice_id) = register(&app, "alice").await;
    let (bob, bob_id) = register(&app, "bob").await;
    let (carol, carol_id) = register(&app, "carol").await;
    let (_dave, dave_id) = register(&app, "dave").await;
    let _erin = register(&app, "erin").await;

    // Bob shares a DM with alice.
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/users/@me/channels",
        &alice,
        Some(json!({"recipient_ids": [bob_id]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Dave shares a group DM with alice.
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/users/@me/channels",
        &alice,
        Some(json!({"recipient_ids": [bob_id, dave_id]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Carol shares a guild with alice.
    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &alice,
        Some(json!({"name": "HQ"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild_id = guild["id"].as_str().expect("guild id").to_owned();
    let (status, channels) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &alice,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let channel_id = channels[0]["id"].as_str().expect("channel id").to_owned();
    let (status, invite) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{channel_id}/invites"),
        &alice,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let code = invite["code"].as_str().expect("code").to_owned();
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/v1/invites/{code}"),
        &carol,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let expected: BTreeSet<String> = [&alice_id, &bob_id, &carol_id, &dave_id]
        .into_iter()
        .cloned()
        .collect();

    // Registration publishes the new device list in the same transaction.
    let (status, device) = call(
        &app,
        Method::POST,
        "/api/v1/devices",
        &alice,
        Some(json!({"name": "laptop", "identity_key_b64": IDENTITY_KEY_B64})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let device_id = device["device_id"].as_str().expect("device id").to_owned();

    let updates = device_updates(&database).await;
    assert_eq!(updates.len(), 1, "one update per registration");
    let registered = &updates[0];
    assert_eq!(registered["data"]["user_id"], alice_id);
    assert_eq!(registered["data"]["devices"][0]["device_id"], device_id);
    assert_eq!(
        registered["data"]["devices"][0]["identity_key_b64"],
        IDENTITY_KEY_B64
    );
    let targets = registered["target_user_ids"].as_array().expect("targets");
    assert_eq!(targets.len(), expected.len(), "targets are de-duplicated");
    assert_eq!(
        target_set(registered),
        expected,
        "erin shares nothing with alice"
    );
    assert_eq!(registered["guild_id"], Value::Null);

    // Only the owner may remove a device.
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/v1/devices/{device_id}"),
        &bob,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        device_updates(&database).await.len(),
        1,
        "no event on refusal"
    );

    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/v1/devices/{device_id}"),
        &alice,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let updates = device_updates(&database).await;
    assert_eq!(updates.len(), 2, "removal publishes an update");
    let removed = &updates[1];
    assert_eq!(removed["data"]["devices"], json!([]));
    assert_eq!(target_set(removed), expected);

    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/v1/devices/{device_id}"),
        &alice,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "already removed");
}
