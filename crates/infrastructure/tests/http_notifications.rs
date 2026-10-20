//! HTTP contract tests for per-guild notification settings (api#28) and private user notes
//! (api#51): defaults, patch semantics, mute windows, scoping, privacy, the outbox events
//! and the internal READY payload the gateway reads.

mod support;

use axum::http::{Method, StatusCode};
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database, Database};

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

fn app(database: &Database) -> axum::Router {
    let (private, public) = jwt_keys();
    interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    })
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

async fn ready(app: &axum::Router, user_id: &str) -> Value {
    let (status, body) = send(
        app,
        axum::http::Request::builder()
            .method("GET")
            .uri(format!("/internal/v1/users/{user_id}/ready"))
            .header("x-internal-token", INTERNAL_TOKEN)
            .body(axum::body::Body::empty())
            .expect("request"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body
}

#[tokio::test]
async fn guild_notification_settings_round_trip_with_overrides_mutes_and_scoping() {
    let database = start_database().await;
    let app = app(&database);
    let (owner, owner_id) = register(&app, "owner@example.com", "owner").await;
    let (stranger, _) = register(&app, "stranger@example.com", "stranger").await;

    let (_, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name": "Voics HQ"})),
    )
    .await;
    let guild_id = guild["id"].as_str().expect("guild id").to_owned();
    let (_, channels) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        None,
    )
    .await;
    let general = channels[0]["id"].as_str().expect("channel").to_owned();
    let path = format!("/api/v1/users/@me/guilds/{guild_id}/settings");

    // Defaults: inherit the guild level, nothing muted.
    let (status, settings) = call(&app, Method::GET, &path, &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settings["guild_id"], json!(guild_id));
    assert_eq!(settings["message_notifications"], Value::Null);
    assert_eq!(settings["muted"], json!(false));
    assert_eq!(settings["mute_config"], Value::Null);
    assert_eq!(settings["channel_overrides"], json!([]));

    // Non-members cannot read or write the settings of a guild.
    let (status, _) = call(&app, Method::GET, &path, &stranger, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&app, Method::PATCH, &path, &stranger, Some(json!({}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &app,
        Method::GET,
        "/api/v1/users/@me/guilds/424242/settings",
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Level, suppression flags, a 1 hour mute window and a channel override in one patch.
    let before = Utc::now();
    let (status, updated) = call(
        &app,
        Method::PATCH,
        &path,
        &owner,
        Some(json!({
            "message_notifications": 1,
            "suppress_everyone": true,
            "suppress_roles": true,
            "muted": true,
            "mute_config": {"selected_time_window": 3600},
            "hide_muted_channels": true,
            "channel_overrides": [
                {"channel_id": general, "message_notifications": 2, "muted": true}
            ],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["message_notifications"], json!(1));
    assert_eq!(updated["suppress_everyone"], json!(true));
    assert_eq!(updated["suppress_roles"], json!(true));
    assert_eq!(updated["hide_muted_channels"], json!(true));
    assert_eq!(updated["muted"], json!(true));
    let end: DateTime<Utc> = updated["mute_config"]["end_time"]
        .as_str()
        .expect("end time")
        .parse()
        .expect("rfc3339");
    assert!(end > before + Duration::minutes(59) && end < before + Duration::minutes(61));
    assert_eq!(
        updated["channel_overrides"][0]["channel_id"],
        json!(general)
    );
    assert_eq!(
        updated["channel_overrides"][0]["message_notifications"],
        json!(2)
    );
    assert_eq!(updated["channel_overrides"][0]["muted"], json!(true));
    // An override muted without a window is indefinite.
    assert_eq!(
        updated["channel_overrides"][0]["mute_config"],
        json!({"end_time": null})
    );

    // The GET returns exactly what was stored.
    let (_, fetched) = call(&app, Method::GET, &path, &owner, None).await;
    assert_eq!(fetched, updated);

    // A partial patch changes only what it names; unmuting clears the end time.
    let (status, partial) = call(
        &app,
        Method::PATCH,
        &path,
        &owner,
        Some(json!({"muted": false, "message_notifications": null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(partial["muted"], json!(false));
    assert_eq!(partial["mute_config"], Value::Null);
    assert_eq!(partial["message_notifications"], Value::Null);
    assert_eq!(partial["suppress_everyone"], json!(true));
    assert_eq!(
        partial["channel_overrides"].as_array().map(Vec::len),
        Some(1)
    );

    // Clearing an override removes it.
    let (_, cleared) = call(
        &app,
        Method::PATCH,
        &path,
        &owner,
        Some(json!({"channel_overrides": [
            {"channel_id": general, "message_notifications": null, "muted": false}
        ]})),
    )
    .await;
    assert_eq!(cleared["channel_overrides"], json!([]));

    // Validation: bad level, bad window, a channel of another guild.
    let (status, _) = call(
        &app,
        Method::PATCH,
        &path,
        &owner,
        Some(json!({"message_notifications": 9})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &app,
        Method::PATCH,
        &path,
        &owner,
        Some(json!({"muted": true, "mute_config": {"selected_time_window": 0}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, other_guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name": "Other"})),
    )
    .await;
    let other_id = other_guild["id"].as_str().expect("id").to_owned();
    let (_, other_channels) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{other_id}/channels"),
        &owner,
        None,
    )
    .await;
    let foreign = other_channels[0]["id"]
        .as_str()
        .expect("channel")
        .to_owned();
    let (status, body) = call(
        &app,
        Method::PATCH,
        &path,
        &owner,
        Some(json!({"channel_overrides": [{"channel_id": foreign, "muted": true}]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // DMs have their own settings under `@me`.
    let (status, dm) = call(
        &app,
        Method::PATCH,
        "/api/v1/users/@me/guilds/@me/settings",
        &owner,
        Some(json!({"muted": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dm["guild_id"], Value::Null);
    assert_eq!(dm["muted"], json!(true));

    // Dispatches target the user's own sessions only.
    let updates = events(&database, "USER_GUILD_SETTINGS_UPDATE").await;
    assert!(updates.len() >= 4, "one event per successful patch");
    for update in &updates {
        assert_eq!(update["target_user_ids"], json!([owner_id]));
    }
    assert_eq!(updates[0]["data"]["suppress_roles"], json!(true));

    // READY carries the stored settings (guild and DMs) and nothing of other guilds' non-members.
    let ready = ready(&app, &owner_id).await;
    let listed = ready["user_guild_settings"].as_array().expect("list");
    assert!(listed.iter().any(|s| s["guild_id"] == json!(guild_id)));
    assert!(listed
        .iter()
        .any(|s| s["guild_id"].is_null() && s["muted"] == json!(true)));
}

#[tokio::test]
async fn user_notes_are_private_validated_and_dispatched_to_the_author_only() {
    let database = start_database().await;
    let app = app(&database);
    let (alice, alice_id) = register(&app, "alice@example.com", "alice").await;
    let (bob, bob_id) = register(&app, "bob@example.com", "bob").await;

    let path = format!("/api/v1/users/@me/notes/{bob_id}");
    let (status, _) = call(&app, Method::GET, &path, &alice, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "no note yet");

    let (status, _) = call(
        &app,
        Method::PUT,
        &path,
        &alice,
        Some(json!({"note": "met at the conference"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, note) = call(&app, Method::GET, &path, &alice, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        note,
        json!({"user_id": alice_id, "note_user_id": bob_id, "note": "met at the conference"})
    );

    // Privacy: the noted user and everyone else see nothing, and bob's own note is separate.
    let (status, _) = call(
        &app,
        Method::GET,
        &format!("/api/v1/users/@me/notes/{alice_id}"),
        &bob,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Replace, then limit: 256 characters pass, 257 are refused.
    let long = "\u{e9}".repeat(256);
    let (status, _) = call(
        &app,
        Method::PUT,
        &path,
        &alice,
        Some(json!({"note": long})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        &app,
        Method::PUT,
        &path,
        &alice,
        Some(json!({"note": "\u{e9}".repeat(257)})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Unknown target user.
    let (status, _) = call(
        &app,
        Method::PUT,
        "/api/v1/users/@me/notes/999999999",
        &alice,
        Some(json!({"note": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // READY carries the author's notes only.
    let alice_ready = ready(&app, &alice_id).await;
    assert_eq!(alice_ready["notes"][&bob_id], json!("\u{e9}".repeat(256)));
    let bob_ready = ready(&app, &bob_id).await;
    assert_eq!(bob_ready["notes"], json!({}));

    // An empty note deletes it; DELETE does too.
    let (status, _) = call(&app, Method::PUT, &path, &alice, Some(json!({"note": ""}))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, Method::GET, &path, &alice, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, _) = call(
        &app,
        Method::PUT,
        &path,
        &alice,
        Some(json!({"note": "again"})),
    )
    .await;
    let (status, _) = call(&app, Method::DELETE, &path, &alice, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, Method::GET, &path, &alice, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Every change was dispatched to the author only; removals carry an empty note.
    let updates = events(&database, "USER_NOTE_UPDATE").await;
    assert_eq!(updates.len(), 5);
    for update in &updates {
        assert_eq!(update["target_user_ids"], json!([alice_id]));
        assert_eq!(update["data"]["id"], json!(bob_id));
    }
    assert_eq!(updates[0]["data"]["note"], json!("met at the conference"));
    assert_eq!(updates[2]["data"]["note"], json!(""));
    assert_eq!(updates[4]["data"]["note"], json!(""));
}
