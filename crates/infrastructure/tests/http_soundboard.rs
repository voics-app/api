mod support;
use axum::http::{Method, StatusCode};
use base64::Engine;
use http_body_util::BodyExt;
use interface::state::AppState;
use serde_json::json;
use support::http::{call, request, send};
use support::{jwt_keys, ports, start_database};
use tower::ServiceExt;

struct ConnectedVoice;
#[async_trait::async_trait]
impl application::ports::PresenceDirectory for ConnectedVoice {
    async fn online_count(&self, _: &[domain::Snowflake]) -> application::AppResult<u64> {
        Ok(0)
    }
    async fn live_ids(
        &self,
        _: &[domain::Snowflake],
    ) -> application::AppResult<Vec<domain::Snowflake>> {
        Ok(vec![])
    }
    async fn can_publish_soundboard(
        &self,
        _: domain::Snowflake,
        _: domain::Snowflake,
        _: domain::Snowflake,
    ) -> application::AppResult<bool> {
        Ok(true)
    }
}

fn wav() -> Vec<u8> {
    let mut data = Vec::from(&b"RIFF"[..]);
    data.extend_from_slice(&38u32.to_le_bytes());
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16u32.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&48_000u32.to_le_bytes());
    data.extend_from_slice(&96_000u32.to_le_bytes());
    data.extend_from_slice(&2u16.to_le_bytes());
    data.extend_from_slice(&16u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&2u32.to_le_bytes());
    data.extend_from_slice(&[0, 0]);
    data
}
async fn register(app: &axum::Router, email: &str, username: &str) -> String {
    let (status, body) = send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({"email":email,"username":username,"password":"a-long-enough-passphrase"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    body["access_token"].as_str().unwrap().to_owned()
}
#[tokio::test]
async fn soundboard_crud_enforces_membership_and_validates_audio() {
    let db = start_database().await;
    let (private, public) = jwt_keys();
    let mut test_ports = ports(&db.pool, &public, &private);
    test_ports.presence = std::sync::Arc::new(ConnectedVoice);
    let app = interface::router(AppState {
        ports: test_ports,
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: Default::default(),
    });
    let owner = register(&app, "sound-owner@example.com", "soundowner").await;
    let outsider = register(&app, "sound-outsider@example.com", "outsider").await;
    let (_, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name":"Sounds"})),
    )
    .await;
    let guild = guild["id"].as_str().unwrap();
    let path = format!("/api/v1/guilds/{guild}/soundboard-sounds");
    let sound = base64::engine::general_purpose::STANDARD.encode(wav());
    let (status, body) = call(
        &app,
        Method::POST,
        &path,
        &owner,
        Some(json!({"name":"horn", "sound":sound})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["id"].as_str().unwrap();
    assert!(body.get("audio").is_none());
    let (_, voice) = call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{guild}/channels"),
        &owner,
        Some(json!({"name":"voice", "type":2})),
    )
    .await;
    let channel = voice["id"].as_str().unwrap();
    let send_path = format!("/api/v1/channels/{channel}/send-soundboard-sound");
    let request_body = json!({"sound_id":id,"source_guild_id":guild});
    // The current gateway session owns one acknowledged audio + soundboard grant; membership and
    // capability are deliberately represented by this one atomic snapshot.
    assert_eq!(
        call(&app, Method::POST, &send_path, &owner, Some(request_body))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let (status, stage) = call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{guild}/channels"),
        &owner,
        Some(json!({"name":"stage", "type":13})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let stage_id = stage["id"].as_str().unwrap();
    let stage_path = format!("/api/v1/channels/{stage_id}/send-soundboard-sound");
    assert_eq!(
        call(
            &app,
            Method::POST,
            &stage_path,
            &owner,
            Some(json!({"sound_id":id, "source_guild_id":guild}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    let detail = format!("{path}/{id}");
    let audio_path = format!("{detail}/audio");
    let response = app
        .clone()
        .oneshot(request(Method::GET, &audio_path, Some(&owner), None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("content-type").unwrap(), "audio/wav");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(bytes.as_ref(), wav());
    assert_eq!(
        call(&app, Method::GET, &audio_path, &outsider, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    assert_eq!(
        call(&app, Method::GET, &detail, &outsider, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            Method::POST,
            &path,
            &owner,
            Some(json!({"name":"bad", "sound":"aGVsbG8="}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, updated) = call(
        &app,
        Method::PATCH,
        &detail,
        &owner,
        Some(json!({"name":"beep", "volume":0.5})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["name"], "beep");
    assert_eq!(
        call(&app, Method::DELETE, &detail, &owner, None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, Method::GET, &detail, &owner, None).await.0,
        StatusCode::NOT_FOUND
    );
    let (events,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM outbox WHERE payload->>'type' LIKE 'SOUNDBOARD_SOUND_%'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(events, 3);
    let (audits,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM audit_log WHERE action IN (130,131,132)")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(audits, 3);
    let sound = base64::engine::general_purpose::STANDARD.encode(wav());
    for index in 0..7 {
        let response = call(
            &app,
            Method::POST,
            &path,
            &owner,
            Some(json!({"name":format!("sound{index}"), "sound":sound})),
        )
        .await;
        assert_eq!(response.0, StatusCode::CREATED);
    }
    let (first, second) = tokio::join!(
        call(
            &app,
            Method::POST,
            &path,
            &owner,
            Some(json!({"name":"race_a", "sound":sound}))
        ),
        call(
            &app,
            Method::POST,
            &path,
            &owner,
            Some(json!({"name":"race_b", "sound":sound}))
        )
    );
    let mut statuses = [first.0.as_u16(), second.0.as_u16()];
    statuses.sort();
    assert_eq!(statuses, [201, 400]);
    let (_, listed) = call(&app, Method::GET, &path, &owner, None).await;
    assert_eq!(listed.as_array().unwrap().len(), 8);
}
