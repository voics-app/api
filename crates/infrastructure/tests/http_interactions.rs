//! HTTP contract tests for the interaction create flow (INTERACTION_CREATE
//! written to the outbox) and the initial response callback route.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database, Database};

struct Fixture {
    app: axum::Router,
    database: Database,
    owner: String,
    application_id: String,
    bot_id: String,
    command_id: String,
    guild_id: String,
    channel_id: String,
}

async fn register(app: &axum::Router, email: &str, username: &str) -> String {
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
    body["access_token"].as_str().expect("token").to_owned()
}

/// A guild with a bot installed, a `ping` slash command and a text channel.
async fn fixture() -> Fixture {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let owner = register(&app, "dev@example.com", "developer").await;

    let (status, created) = call(
        &app,
        Method::POST,
        "/api/v1/applications",
        &owner,
        Some(json!({"name": "Helper Bot", "description": "Helps"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let application_id = created["id"].as_str().expect("application id").to_owned();
    let bot_id = created["bot"]["id"].as_str().expect("bot id").to_owned();

    let (status, command) = call(
        &app,
        Method::POST,
        &format!("/api/v1/applications/{application_id}/commands"),
        &owner,
        Some(json!({"name": "ping", "description": "Replies pong", "type": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let command_id = command["id"].as_str().expect("command id").to_owned();

    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name": "Bots"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild_id = guild["id"].as_str().expect("guild id").to_owned();

    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/oauth2/authorize",
        &owner,
        Some(json!({
            "client_id": application_id,
            "guild_id": guild_id,
            "permissions": "8",
            "scope": "bot applications.commands"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "the owner installs the bot");

    let (status, channels) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let channel_id = channels
        .as_array()
        .and_then(|list| list.iter().find(|c| c["type"] == 0))
        .and_then(|c| c["id"].as_str())
        .expect("a text channel")
        .to_owned();

    Fixture {
        app,
        database,
        owner,
        application_id,
        bot_id,
        command_id,
        guild_id,
        channel_id,
    }
}

async fn invoke_ping(fixture: &Fixture) -> (StatusCode, Value) {
    call(
        &fixture.app,
        Method::POST,
        "/api/v1/interactions",
        &fixture.owner,
        Some(json!({
            "application_id": fixture.application_id,
            "type": 2,
            "channel_id": fixture.channel_id,
            "data": {"id": fixture.command_id, "name": "ping", "type": 1}
        })),
    )
    .await
}

/// Payloads of every INTERACTION_CREATE row in the outbox.
async fn interaction_create_events(pool: &sqlx::PgPool) -> Vec<Value> {
    let rows: Vec<(Value,)> = sqlx::query_as(
        "SELECT payload FROM outbox WHERE payload->>'type' = 'INTERACTION_CREATE' ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .expect("outbox query");
    rows.into_iter().map(|(payload,)| payload).collect()
}

async fn interaction_state(pool: &sqlx::PgPool, id: &str) -> String {
    let (state,): (String,) = sqlx::query_as("SELECT state FROM interactions WHERE id = $1")
        .bind(id.parse::<i64>().expect("snowflake"))
        .fetch_one(pool)
        .await
        .expect("interaction row");
    state
}

#[tokio::test]
async fn invoking_a_command_writes_interaction_create_to_the_outbox() {
    let fixture = fixture().await;
    let (status, created) = invoke_ping(&fixture).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let interaction_id = created["id"].as_str().expect("interaction id").to_owned();

    let events = interaction_create_events(&fixture.database.pool).await;
    assert_eq!(events.len(), 1, "exactly one INTERACTION_CREATE");
    let event = &events[0];
    assert_eq!(event["target_user_ids"], json!([fixture.bot_id]));
    assert_eq!(event["guild_id"], json!(fixture.guild_id));
    assert_eq!(event["channel_id"], json!(fixture.channel_id));

    let data = &event["data"];
    assert_eq!(data["id"], json!(interaction_id));
    assert_eq!(data["application_id"], json!(fixture.application_id));
    assert_eq!(data["type"], 2);
    assert_eq!(data["data"]["id"], json!(fixture.command_id));
    assert!(
        data["token"].as_str().is_some_and(|t| !t.is_empty()),
        "the event carries the interaction token"
    );
    assert_eq!(
        interaction_state(&fixture.database.pool, &interaction_id).await,
        "pending"
    );
}

#[tokio::test]
async fn callback_with_the_token_acknowledges_the_interaction() {
    let fixture = fixture().await;
    let (_, created) = invoke_ping(&fixture).await;
    let interaction_id = created["id"].as_str().expect("interaction id").to_owned();
    let events = interaction_create_events(&fixture.database.pool).await;
    let token = events[0]["data"]["token"]
        .as_str()
        .expect("token")
        .to_owned();

    let (status, _) = send(
        &fixture.app,
        request(
            Method::POST,
            &format!("/api/v1/interactions/{interaction_id}/{token}/callback"),
            None,
            Some(json!({"type": 5})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        interaction_state(&fixture.database.pool, &interaction_id).await,
        "deferred"
    );

    let (status, _) = send(
        &fixture.app,
        request(
            Method::POST,
            &format!("/api/v1/interactions/{interaction_id}/{token}/callback"),
            None,
            Some(json!({"type": 5})),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a second initial response is rejected"
    );
}

#[tokio::test]
async fn callback_with_a_wrong_token_is_not_found_and_changes_nothing() {
    let fixture = fixture().await;
    let (_, created) = invoke_ping(&fixture).await;
    let interaction_id = created["id"].as_str().expect("interaction id").to_owned();

    let (status, body) = send(
        &fixture.app,
        request(
            Method::POST,
            &format!("/api/v1/interactions/{interaction_id}/not-the-token/callback"),
            None,
            Some(json!({"type": 5})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        interaction_state(&fixture.database.pool, &interaction_id).await,
        "pending"
    );
}

#[tokio::test]
async fn callback_after_the_token_expired_is_not_found() {
    let fixture = fixture().await;
    let (_, created) = invoke_ping(&fixture).await;
    let interaction_id = created["id"].as_str().expect("interaction id").to_owned();
    let events = interaction_create_events(&fixture.database.pool).await;
    let token = events[0]["data"]["token"]
        .as_str()
        .expect("token")
        .to_owned();

    sqlx::query(
        "UPDATE interactions SET created_at = created_at - interval '16 minutes' WHERE id = $1",
    )
    .bind(interaction_id.parse::<i64>().expect("snowflake"))
    .execute(&fixture.database.pool)
    .await
    .expect("age the interaction");

    let (status, body) = send(
        &fixture.app,
        request(
            Method::POST,
            &format!("/api/v1/interactions/{interaction_id}/{token}/callback"),
            None,
            Some(json!({"type": 5})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        interaction_state(&fixture.database.pool, &interaction_id).await,
        "pending",
        "an expired callback leaves the interaction untouched"
    );
}
