//! End-to-end ban appeal HTTP flow.

mod support;

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use serde_json::json;

use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{
    jwt_keys, ports_with_messages, start_database, BudgetLimiter, Database, RecordingMessages,
};

struct World {
    app: axum::Router,
    guild_id: String,
    owner: String,
    ann: String,
    ann_id: String,
    ben: String,
}

fn router(
    database: &Database,
    messages: Arc<RecordingMessages>,
    budget: Option<u32>,
) -> axum::Router {
    let (private, public) = jwt_keys();
    let mut ports = ports_with_messages(&database.pool, &public, &private, messages);
    if let Some(budget) = budget {
        ports.limiter = Arc::new(BudgetLimiter::new(budget));
    }
    interface::router(AppState {
        ports,
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    })
}

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

/// A guild with an owner and two members, plus a text channel.
async fn world(database: &Database, budget: Option<u32>) -> World {
    let messages = Arc::new(RecordingMessages::default());
    let app = router(database, messages.clone(), budget);
    let (owner, _owner_id) = register(&app, "owner").await;
    let (ann, ann_id) = register(&app, "ann").await;
    let (ben, _ben_id) = register(&app, "ben").await;
    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name": "HQ"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild_id = guild["id"].as_str().expect("guild id").to_owned();
    let (_, channels) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        None,
    )
    .await;
    let channel_id = channels
        .as_array()
        .expect("channels")
        .iter()
        .find(|c| c["type"] == 0)
        .expect("text channel")["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let (_, invite) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{channel_id}/invites"),
        &owner,
        Some(json!({})),
    )
    .await;
    let code = invite["code"].as_str().expect("code").to_owned();
    for member in [&ann, &ben] {
        let (status, _) = call(
            &app,
            Method::POST,
            &format!("/api/v1/invites/{code}"),
            member,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    World {
        app,
        guild_id,
        owner,
        ann,
        ann_id,
        ben,
    }
}

#[tokio::test]
async fn banned_user_submits_and_moderator_accepts_appeal() {
    let database = start_database().await;
    let w = world(&database, None).await;
    let base = format!("/api/v1/guilds/{}/ban-appeals", w.guild_id);
    let ban = format!("/api/v1/guilds/{}/bans/{}", w.guild_id, w.ann_id);
    assert_eq!(
        call(&w.app, Method::PUT, &ban, &w.owner, None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &w.app,
            Method::PUT,
            &format!("{base}/form"),
            &w.owner,
            Some(json!({"question":"Why should we unban you?"}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let (status, form) = call(&w.app, Method::GET, &format!("{base}/form"), &w.ann, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(form["question"], "Why should we unban you?");
    let (status, appeal) = call(
        &w.app,
        Method::POST,
        &base,
        &w.ann,
        Some(json!({"answer":"I understand the rules."})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        call(
            &w.app,
            Method::POST,
            &base,
            &w.ann,
            Some(json!({"answer":"again"}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let id = appeal["id"].as_str().unwrap();
    assert_eq!(
        call(&w.app, Method::GET, &base, &w.ben, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &w.app,
            Method::POST,
            &format!("{base}/{id}/accept"),
            &w.owner,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&w.app, Method::GET, &ban, &w.owner, None).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, audit) = call(
        &w.app,
        Method::GET,
        &format!("/api/v1/guilds/{}/audit-logs", w.guild_id),
        &w.owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(audit["audit_log_entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["action_type"] == 200));
}
