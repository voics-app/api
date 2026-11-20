mod support;

use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{Method, StatusCode};
use domain::Snowflake;
use interface::state::AppState;
use serde_json::json;
use support::http::{call, request, send};
use support::{jwt_keys, ports, start_database};

struct VoicePresence {
    user: Snowflake,
    channel: Snowflake,
}

#[async_trait]
impl application::ports::PresenceDirectory for VoicePresence {
    async fn online_count(&self, _: &[Snowflake]) -> application::AppResult<u64> {
        Ok(0)
    }
    async fn live_ids(&self, _: &[Snowflake]) -> application::AppResult<Vec<Snowflake>> {
        Ok(vec![])
    }
    async fn in_voice_channel(
        &self,
        _: Snowflake,
        channel: Snowflake,
        user: Snowflake,
    ) -> application::AppResult<bool> {
        Ok(user == self.user && channel == self.channel)
    }
}

async fn register(app: &axum::Router, name: &str) -> (String, String) {
    let (status, body) = send(app, request(Method::POST, "/api/v1/auth/register", None,
        Some(json!({"email": format!("{name}@example.com"), "username": name, "password": "a-long-enough-passphrase"})))).await;
    assert_eq!(status, StatusCode::CREATED);
    (
        body["access_token"].as_str().unwrap().to_owned(),
        body["user"]["id"].as_str().unwrap().to_owned(),
    )
}

#[tokio::test]
async fn voice_status_checks_access_and_preserves_other_channel_fields() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let mut app_ports = ports(&database.pool, &public, &private);
    let mk_app = |ports| {
        interface::router(AppState {
            ports,
            internal_token: support::http::INTERNAL_TOKEN.into(),
            rate_limiter: Arc::new(support::AllowAll),
            rate_policy: interface::state::RatePolicy::default(),
        })
    };
    let app = mk_app(app_ports.clone());
    let (owner, _) = register(&app, "voiceowner").await;
    let (guest, guest_id) = register(&app, "voiceguest").await;
    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name":"Voice HQ"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild_id = guild["id"].as_str().unwrap();
    let (status, voice) = call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        Some(json!({"name":"voice-room", "type":2})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let voice_id = voice["id"].as_str().unwrap();
    let (status, text) = call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        Some(json!({"name":"text-room", "type":0})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let text_id = text["id"].as_str().unwrap();
    let (status, invite) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{text_id}/invites"),
        &owner,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let code = invite["code"].as_str().unwrap();
    assert_eq!(
        call(
            &app,
            Method::POST,
            &format!("/api/v1/invites/{code}"),
            &guest,
            None
        )
        .await
        .0,
        StatusCode::OK
    );

    assert_eq!(
        call(
            &app,
            Method::PUT,
            &format!("/api/v1/channels/{voice_id}/voice-status"),
            &guest,
            Some(json!({"status":"must not be saved"})),
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE,
        "an unavailable gateway does not grant connected-member access"
    );

    app_ports.presence = Arc::new(VoicePresence {
        user: guest_id.parse().unwrap(),
        channel: voice_id.parse().unwrap(),
    });
    let app = mk_app(app_ports.clone());
    let insights_path = format!("/api/v1/guilds/{guild_id}/insights?period=7");
    assert_eq!(
        call(&app, Method::GET, &insights_path, &guest, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, insights) = call(&app, Method::GET, &insights_path, &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    let days = insights["days"].as_array().unwrap();
    assert_eq!(days.len(), 7);
    assert_eq!(days.last().unwrap()["new_members"], 2);
    assert_eq!(days.last().unwrap()["membership_complete"], false);
    assert!(days.last().unwrap()["activity_source_initialized_at_ms"].is_i64());
    assert!(days.last().unwrap()["messages"].is_null());
    assert!(days.last().unwrap()["voice_minutes"].is_null());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream = axum::Router::new().route(
        "/internal/v1/guilds/{guild}/message-activity/{day}",
        axum::routing::get(
            |axum::extract::Path((_guild, day)): axum::extract::Path<(String, i64)>,
             headers: axum::http::HeaderMap| async move {
                assert_eq!(
                    headers.get("x-internal-token").unwrap(),
                    support::http::INTERNAL_TOKEN
                );
                axum::Json(json!({"utc_day": day, "count": 4, "coverage_complete": false, "source_initialized_at_ms": 1795046400000_u64}))
            },
        ),
    );
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let mut with_messages = app_ports.clone();
    with_messages.messages = Arc::new(infrastructure::messages_client::HttpMessages::new(
        format!("http://{addr}/").parse().unwrap(),
        support::http::INTERNAL_TOKEN.into(),
    ));
    let with_messages_app = mk_app(with_messages);
    let (status, observed) = call(
        &with_messages_app,
        Method::GET,
        &insights_path,
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(observed["days"][6]["messages"], 4);
    assert_eq!(observed["days"][6]["message_complete"], false);
    assert_eq!(
        observed["days"][6]["message_source_initialized_at_ms"],
        1795046400000_u64
    );
    assert_eq!(observed["message_coverage"], false);
    server.abort();
    let _ = server.await;
    assert_eq!(
        call(
            &app,
            Method::GET,
            &format!("/api/v1/guilds/{guild_id}/insights?period=8"),
            &owner,
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let widget_path = format!("/api/v1/guilds/{guild_id}/widget.json");
    assert_eq!(
        send(&app, request(Method::GET, &widget_path, None, None))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let settings_path = format!("/api/v1/guilds/{guild_id}/widget");
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &settings_path,
            &guest,
            Some(json!({"enabled":true}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &settings_path,
            &owner,
            Some(json!({"enabled":null}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, settings) = call(
        &app,
        Method::PATCH,
        &settings_path,
        &owner,
        Some(json!({"enabled":true, "channel_id":text_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settings["channel_id"], text_id);
    let (status, public_widget) = send(&app, request(Method::GET, &widget_path, None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(public_widget["id"], guild_id);
    assert!(public_widget.get("channel_id").is_none());
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &settings_path,
            &owner,
            Some(json!({"channel_id":"999999"}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &settings_path,
            &owner,
            Some(json!({"enabled":false}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        send(&app, request(Method::GET, &widget_path, None, None))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let path = format!("/api/v1/channels/{voice_id}/voice-status");
    assert_eq!(
        call(
            &app,
            Method::PUT,
            &path,
            &guest,
            Some(json!({"status":"Listening"}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let (_, current) = call(
        &app,
        Method::GET,
        &format!("/api/v1/channels/{voice_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(current["status"], "Listening");
    assert_eq!(current["name"], "voice-room");
    assert_eq!(
        call(
            &app,
            Method::PUT,
            &path,
            &owner,
            Some(json!({"status":"x".repeat(501)}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &app,
            Method::PUT,
            &format!("/api/v1/channels/{text_id}/voice-status"),
            &owner,
            Some(json!({"status":"x"}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &app,
            Method::PUT,
            &path,
            &owner,
            Some(json!({"status":null}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let (_, current) = call(
        &app,
        Method::GET,
        &format!("/api/v1/channels/{voice_id}"),
        &owner,
        None,
    )
    .await;
    assert!(current["status"].is_null());
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE payload::text LIKE '%CHANNEL_UPDATE%' AND payload::text LIKE '%Listening%'")
        .fetch_one(&database.pool).await.unwrap();
    assert_eq!(events, 1);

    let guild_number: i64 = guild_id.parse().unwrap();
    let counts: (i64, i64) =
        sqlx::query_as("SELECT joins, leaves FROM guild_daily_activity WHERE guild_id = $1")
            .bind(guild_number)
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(counts, (2, 0));
    let mut tx = database.pool.begin().await.unwrap();
    sqlx::query("DELETE FROM members WHERE guild_id = $1 AND user_id = $2")
        .bind(guild_number)
        .bind(guest_id.parse::<i64>().unwrap())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    let counts: (i64, i64) =
        sqlx::query_as("SELECT joins, leaves FROM guild_daily_activity WHERE guild_id = $1")
            .bind(guild_number)
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(counts, (2, 0), "rolled-back departures are not counted");
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/v1/users/@me/guilds/{guild_id}"),
            &guest,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let counts: (i64, i64) =
        sqlx::query_as("SELECT joins, leaves FROM guild_daily_activity WHERE guild_id = $1")
            .bind(guild_number)
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(counts, (2, 1));

    let checkpoint_body = json!({
        "guild_id": guild_id, "user_id": guest_id, "session_id": "http-session",
        "generation_id": uuid::Uuid::new_v4(), "observed_at": chrono::Utc::now(), "connected": true
    });
    let checkpoint_request = |token: Option<&str>| {
        let mut builder = axum::http::Request::builder()
            .method(Method::POST)
            .uri("/internal/v1/voice/activity-checkpoints")
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header("x-internal-token", token);
        }
        builder
            .body(axum::body::Body::from(checkpoint_body.to_string()))
            .unwrap()
    };
    assert_eq!(
        send(&app, checkpoint_request(None)).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(
            &app,
            checkpoint_request(Some(support::http::INTERNAL_TOKEN))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );

    let midnight = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    let mut checkpoint = application::ports::VoiceCheckpoint {
        guild_id: guild_id.parse().unwrap(),
        user_id: guest_id.parse().unwrap(),
        session_id: "voice-session".into(),
        generation_id: uuid::Uuid::new_v4(),
        observed_at: midnight - chrono::Duration::seconds(10),
        connected: true,
    };
    app_ports
        .guilds
        .voice_checkpoint(checkpoint.clone())
        .await
        .unwrap();
    checkpoint.observed_at = midnight + chrono::Duration::seconds(10);
    app_ports
        .guilds
        .voice_checkpoint(checkpoint.clone())
        .await
        .unwrap();
    app_ports
        .guilds
        .voice_checkpoint(checkpoint.clone())
        .await
        .unwrap();
    let voice: Vec<(chrono::NaiveDate, i64)> = sqlx::query_as("SELECT day, voice_millis FROM guild_daily_activity WHERE guild_id=$1 AND voice_millis > 0 ORDER BY day")
        .bind(guild_number).fetch_all(&database.pool).await.unwrap();
    assert_eq!(voice.len(), 2);
    assert_eq!(voice[0].1, 10_000);
    assert_eq!(voice[1].1, 10_000);
    checkpoint.observed_at += chrono::Duration::minutes(2);
    checkpoint.connected = false;
    app_ports.guilds.voice_checkpoint(checkpoint).await.unwrap();
    let total: i64 = sqlx::query_scalar(
        "SELECT sum(voice_millis)::bigint FROM guild_daily_activity WHERE guild_id=$1",
    )
    .bind(guild_number)
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(total, 20_000, "long unobserved gaps are not invented");
}
