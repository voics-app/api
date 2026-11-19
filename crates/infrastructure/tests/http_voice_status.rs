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

    app_ports.presence = Arc::new(VoicePresence {
        user: guest_id.parse().unwrap(),
        channel: voice_id.parse().unwrap(),
    });
    let app = mk_app(app_ports);
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
}
