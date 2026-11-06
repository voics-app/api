//! HTTP contract tests for reporting messages and users (api#126): validation, the content
//! snapshot for stored messages, reporter-submitted content for ephemeral chats, deduplication,
//! the per-user rate limit, and the admin review queue (`/internal/v1/admin/reports`).

mod support;

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use domain::Snowflake;
use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{
    jwt_keys, ports_with_messages, start_database, BudgetLimiter, Database, RecordingMessages,
};

const MESSAGE_ID: &str = "7001";

struct World {
    app: axum::Router,
    messages: Arc<RecordingMessages>,
    guild_id: String,
    channel_id: String,
    owner: String,
    owner_id: String,
    ann: String,
    ann_id: String,
    ben: String,
    ben_id: String,
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
    let (owner, owner_id) = register(&app, "owner").await;
    let (ann, ann_id) = register(&app, "ann").await;
    let (ben, ben_id) = register(&app, "ben").await;
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
        messages,
        guild_id,
        channel_id,
        owner,
        owner_id,
        ann,
        ann_id,
        ben,
        ben_id,
    }
}

impl World {
    /// Makes the messages stub serve a message written by `author_id`.
    fn store_message(&self, author_id: &str, content: &str) {
        let channel: Snowflake = self.channel_id.parse().unwrap();
        let message: Snowflake = MESSAGE_ID.parse().unwrap();
        self.messages.stored.lock().unwrap().insert(
            (channel, message),
            json!({"id": MESSAGE_ID, "content": content, "author": {"id": author_id}}),
        );
    }

    async fn report(&self, token: &str, body: Value) -> (StatusCode, Value) {
        call(
            &self.app,
            Method::POST,
            "/api/v1/reports",
            token,
            Some(body),
        )
        .await
    }

    fn message_report(&self, category: &str) -> Value {
        json!({
            "target": "message",
            "category": category,
            "channel_id": self.channel_id,
            "message_id": MESSAGE_ID,
        })
    }

    async fn queue(&self, query: &str) -> Value {
        let (status, page) = send(
            &self.app,
            internal(
                Method::GET,
                &format!("/internal/v1/admin/reports{query}"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        page
    }
}

fn internal(
    method: Method,
    path: &str,
    body: Option<Value>,
) -> axum::http::Request<axum::body::Body> {
    let builder = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("x-internal-token", INTERNAL_TOKEN);
    match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .expect("request"),
        None => builder.body(axum::body::Body::empty()).expect("request"),
    }
}

#[tokio::test]
async fn a_message_report_snapshots_the_stored_content_for_the_queue() {
    let database = start_database().await;
    let w = world(&database, None).await;
    w.store_message(&w.ann_id, "buy my coins");

    let mut body = w.message_report("spam");
    body["note"] = json!("  posts this everywhere ");
    // Content sent by the reporter is ignored when the server holds the message.
    body["content"] = json!("something else entirely");
    let (status, response) = w.report(&w.ben, body).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(response, Value::Null, "the reporter gets nothing back");

    let page = w.queue("").await;
    let report = &page["reports"][0];
    assert_eq!(report["kind"], "message");
    assert_eq!(report["category"], "spam");
    assert_eq!(report["note"], "posts this everywhere");
    assert_eq!(report["status"], "open");
    assert_eq!(report["content"], "buy my coins");
    assert_eq!(report["content_source"], "server");
    assert_eq!(report["delivery_mode"], "standard");
    assert_eq!(report["reported"]["id"], w.ann_id);
    assert_eq!(report["reported"]["username"], "ann");
    assert_eq!(report["reporter"]["id"], w.ben_id);
    assert_eq!(report["channel_id"], w.channel_id);
    assert_eq!(report["guild_id"], w.guild_id);
    assert_eq!(report["message_id"], MESSAGE_ID);

    // A later edit or delete does not change what was reported.
    w.store_message(&w.ann_id, "edited afterwards");
    assert_eq!(w.queue("").await["reports"][0]["content"], "buy my coins");
}

#[tokio::test]
async fn repeating_an_open_report_is_a_silent_no_op() {
    let database = start_database().await;
    let w = world(&database, None).await;
    w.store_message(&w.ann_id, "rude");

    for _ in 0..2 {
        let (status, _) = w.report(&w.ben, w.message_report("harassment")).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    assert_eq!(w.queue("").await["reports"].as_array().unwrap().len(), 1);

    let (status, _) = w.report(&w.owner, w.message_report("harassment")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        w.queue("").await["reports"].as_array().unwrap().len(),
        2,
        "another reporter files a second report"
    );
}

#[tokio::test]
async fn invalid_message_reports_are_refused_without_a_trace() {
    let database = start_database().await;
    let w = world(&database, None).await;
    w.store_message(&w.ann_id, "hello");

    let (status, body) = w.report(&w.ben, w.message_report("annoying")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], 50035);

    let mut long_note = w.message_report("spam");
    long_note["note"] = json!("x".repeat(501));
    assert_eq!(w.report(&w.ben, long_note).await.0, StatusCode::BAD_REQUEST);

    assert_eq!(
        w.report(&w.ann, w.message_report("spam")).await.0,
        StatusCode::BAD_REQUEST,
        "your own message"
    );

    let mut unknown = w.message_report("spam");
    unknown["message_id"] = json!("9999");
    assert_eq!(w.report(&w.ben, unknown).await.0, StatusCode::NOT_FOUND);

    let mut malformed = w.message_report("spam");
    malformed["message_id"] = json!("not-an-id");
    assert_eq!(w.report(&w.ben, malformed).await.0, StatusCode::BAD_REQUEST);

    let (outsider, _) = register(&w.app, "outsider").await;
    let (status, _) = w.report(&outsider, w.message_report("spam")).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "cannot report what you cannot see"
    );

    let (status, _) = send(
        &w.app,
        request(
            Method::POST,
            "/api/v1/reports",
            None,
            Some(w.message_report("spam")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    assert!(w.queue("").await["reports"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn ephemeral_reports_carry_only_what_the_reporter_submits() {
    let database = start_database().await;
    let w = world(&database, None).await;
    let (status, _) = call(
        &w.app,
        Method::PATCH,
        &format!("/api/v1/channels/{}", w.channel_id),
        &w.owner,
        Some(json!({"delivery_mode": "ephemeral"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // The server holds nothing for this message; the stub proves it is never asked.
    let mut body = w.message_report("harassment");

    assert_eq!(
        w.report(&w.ben, body.clone()).await.0,
        StatusCode::BAD_REQUEST,
        "the author is required when the server cannot look it up"
    );

    body["author_id"] = json!(w.ann_id);
    body["content"] = json!("  what I saw  ");
    assert_eq!(
        w.report(&w.ben, body.clone()).await.0,
        StatusCode::NO_CONTENT
    );
    let report = w.queue("").await["reports"][0].clone();
    assert_eq!(report["content"], "what I saw");
    assert_eq!(report["content_source"], "reporter");
    assert_eq!(report["delivery_mode"], "ephemeral");
    assert_eq!(report["reported"]["id"], w.ann_id);

    let (stranger, stranger_id) = register(&w.app, "stranger").await;
    let _ = stranger;
    let mut outside = body.clone();
    outside["author_id"] = json!(stranger_id);
    assert_eq!(
        w.report(&w.owner, outside).await.0,
        StatusCode::BAD_REQUEST,
        "the author must be in the channel"
    );

    let mut bare = w.message_report("spam");
    bare["author_id"] = json!(w.owner_id);
    bare["message_id"] = json!("7002");
    assert_eq!(w.report(&w.ben, bare).await.0, StatusCode::NO_CONTENT);
    let page = w.queue("").await;
    let bare_report = page["reports"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["reported"]["id"] == w.owner_id)
        .expect("report without content");
    assert_eq!(bare_report["content"], Value::Null);
    assert_eq!(bare_report["content_source"], Value::Null);

    let mut oversized = body;
    oversized["content"] = json!("y".repeat(4001));
    assert_eq!(
        w.report(&w.owner, oversized).await.0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn users_can_be_reported_but_not_yourself() {
    let database = start_database().await;
    let w = world(&database, None).await;
    let user =
        |id: &str, category: &str| json!({"target": "user", "user_id": id, "category": category});

    assert_eq!(
        w.report(&w.ben, user(&w.ann_id, "impersonation")).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        w.report(&w.ben, user(&w.ann_id, "impersonation")).await.0,
        StatusCode::NO_CONTENT,
        "repeat is a no-op"
    );
    let page = w.queue("").await;
    assert_eq!(page["reports"].as_array().unwrap().len(), 1);
    assert_eq!(page["reports"][0]["kind"], "user");
    assert_eq!(page["reports"][0]["content"], Value::Null);
    assert_eq!(page["reports"][0]["message_id"], Value::Null);

    assert_eq!(
        w.report(&w.ben, user(&w.ben_id, "spam")).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        w.report(&w.ben, user("424242", "spam")).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        w.report(&w.ben, json!({"target": "channel", "category": "spam"}))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn reporters_are_rate_limited() {
    let database = start_database().await;
    let w = world(&database, Some(2)).await;
    let body = |id: &str| json!({"target": "user", "user_id": id, "category": "spam"});

    assert_eq!(
        w.report(&w.ben, body(&w.ann_id)).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        w.report(&w.ben, body(&w.owner_id)).await.0,
        StatusCode::NO_CONTENT
    );
    let (status, limited) = w.report(&w.ben, body(&w.ann_id)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(limited["retry_after"].is_number());
    assert_eq!(
        w.report(&w.ann, body(&w.owner_id)).await.0,
        StatusCode::NO_CONTENT,
        "the budget is per reporter"
    );
}

#[tokio::test]
async fn the_queue_lists_by_status_and_reviewers_close_reports_once() {
    let database = start_database().await;
    let w = world(&database, None).await;
    for target in [&w.ann_id, &w.owner_id] {
        let (status, _) = w
            .report(
                &w.ben,
                json!({"target": "user", "user_id": target, "category": "other"}),
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    let page = w.queue("?limit=1").await;
    assert_eq!(page["reports"].as_array().unwrap().len(), 1);
    let first = page["reports"][0]["id"].as_str().unwrap().to_owned();
    let cursor = page["next_after"]
        .as_str()
        .expect("a second page")
        .to_owned();
    let older = w.queue(&format!("?limit=1&after={cursor}")).await;
    assert_eq!(older["next_after"], Value::Null);
    let second = older["reports"][0]["id"].as_str().unwrap().to_owned();
    assert_ne!(first, second);

    let resolve = |id: &str, body: Value| {
        internal(
            Method::POST,
            &format!("/internal/v1/admin/reports/{id}/resolve"),
            Some(body),
        )
    };
    let (status, closed) = send(
        &w.app,
        resolve(
            &first,
            json!({"resolution": "resolved", "resolved_by": "ops@voics.example", "note": " warned "}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(closed["status"], "resolved");
    assert_eq!(closed["resolved_by"], "ops@voics.example");
    assert_eq!(closed["resolution_note"], "warned");
    assert!(closed["resolved_at"].is_string());

    let (status, _) = send(
        &w.app,
        resolve(
            &first,
            json!({"resolution": "dismissed", "resolved_by": "ops@voics.example"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "a closed report stays closed");

    let (status, dismissed) = send(
        &w.app,
        resolve(
            &second,
            json!({"resolution": "dismissed", "resolved_by": "ops@voics.example"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dismissed["status"], "dismissed");

    for (id, body) in [
        (
            second.as_str(),
            json!({"resolution": "open", "resolved_by": "ops"}),
        ),
        ("1", json!({"resolution": "resolved", "resolved_by": "ops"})),
        (
            "nope",
            json!({"resolution": "resolved", "resolved_by": "ops"}),
        ),
    ] {
        let (status, _) = send(&w.app, resolve(id, body)).await;
        assert!(
            status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND,
            "{id}: {status}"
        );
    }

    assert!(w.queue("").await["reports"].as_array().unwrap().is_empty());
    assert_eq!(w.queue("?status=resolved").await["reports"][0]["id"], first);
    assert_eq!(
        w.queue("?status=dismissed").await["reports"][0]["id"],
        second
    );
    let (status, _) = send(
        &w.app,
        internal(Method::GET, "/internal/v1/admin/reports?status=bogus", None),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = send(
        &w.app,
        request(Method::GET, "/internal/v1/admin/reports", None, None),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the queue needs the internal token"
    );
}
