//! HTTP contract tests for the login sessions list and remote logout: revoking
//! invalidates tokens at once and publishes SESSION_REVOKE for the gateway.

mod support;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database, Database};

const DESKTOP_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Discordy/0.4.2 Chrome/130.0.0.0 Electron/33.0.0 Safari/537.36";
const FIREFOX_UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0";
const PASSWORD: &str = "a-long-enough-passphrase";

struct Login {
    access: String,
    refresh: String,
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

fn tokens(body: &Value) -> Login {
    Login {
        access: body["access_token"].as_str().expect("access").to_owned(),
        refresh: body["refresh_token"].as_str().expect("refresh").to_owned(),
    }
}

async fn register(app: &axum::Router, username: &str) -> Login {
    let (status, body) = send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({
                "email": format!("{username}@example.com"),
                "username": username,
                "password": PASSWORD,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "register {username}");
    tokens(&body)
}

async fn login(app: &axum::Router, username: &str, user_agent: &str, country: &str) -> Login {
    let http = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/auth/login")
        .header("content-type", "application/json")
        .header("user-agent", user_agent)
        .header("cf-ipcountry", country)
        .body(Body::from(
            json!({ "email": format!("{username}@example.com"), "password": PASSWORD }).to_string(),
        ))
        .expect("request");
    let (status, body) = send(app, http).await;
    assert_eq!(status, StatusCode::OK, "login {username}");
    tokens(&body)
}

async fn sessions(app: &axum::Router, token: &str) -> Vec<Value> {
    let (status, body) = call(app, Method::GET, "/api/v1/users/@me/sessions", token, None).await;
    assert_eq!(status, StatusCode::OK);
    body.as_array().expect("array").clone()
}

fn current_id(list: &[Value]) -> String {
    list.iter()
        .find(|session| session["current"] == true)
        .expect("a current session")["id"]
        .as_str()
        .expect("id")
        .to_owned()
}

async fn status_of(app: &axum::Router, token: &str) -> StatusCode {
    call(app, Method::GET, "/api/v1/users/@me", token, None)
        .await
        .0
}

async fn delete_session(app: &axum::Router, token: &str, id: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::DELETE,
        &format!("/api/v1/users/@me/sessions/{id}"),
        token,
        None,
    )
    .await
}

async fn revocations(database: &Database) -> Vec<Value> {
    let rows: Vec<(Value,)> = sqlx::query_as(
        "SELECT payload FROM outbox WHERE payload->>'type' = 'SESSION_REVOKE' ORDER BY id",
    )
    .fetch_all(&database.pool)
    .await
    .expect("outbox query");
    rows.into_iter().map(|(payload,)| payload).collect()
}

#[tokio::test]
async fn lists_live_sessions_with_client_details_and_marks_the_current_one() {
    let database = start_database().await;
    let app = app(&database);
    let _registered = register(&app, "alice").await;
    let desktop = login(&app, "alice", DESKTOP_UA, "NL").await;
    let browser = login(&app, "alice", FIREFOX_UA, "XX").await;

    let list = sessions(&app, &browser.access).await;

    assert_eq!(list.len(), 3);
    assert_eq!(list[0]["current"], true, "most recently used first");
    assert_eq!(list[0]["client"], "Firefox");
    assert_eq!(list[0]["os"], "Linux");
    assert_eq!(list[0]["location"], Value::Null);
    let desktop_session = list
        .iter()
        .find(|s| s["client"] == "Desktop app")
        .expect("desktop session");
    assert_eq!(desktop_session["os"], "Windows");
    assert_eq!(desktop_session["location"], "NL");
    assert_eq!(desktop_session["current"], false);
    assert!(desktop_session["last_used_at"].is_string());
    assert!(desktop_session["created_at"].is_string());
    assert_eq!(sessions(&app, &desktop.access).await.len(), 3);
}

#[tokio::test]
async fn revoking_a_session_invalidates_its_tokens_and_publishes_session_revoke() {
    let database = start_database().await;
    let app = app(&database);
    let first = register(&app, "bob").await;
    let second = login(&app, "bob", DESKTOP_UA, "NL").await;
    let second_id = current_id(&sessions(&app, &second.access).await);
    let (_, me) = call(&app, Method::GET, "/api/v1/users/@me", &first.access, None).await;
    let user_id = me["id"].as_str().expect("id").to_owned();

    let (status, _) = delete_session(&app, &first.access, &second_id).await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        status_of(&app, &second.access).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(status_of(&app, &first.access).await, StatusCode::OK);
    let (refresh_status, _) = send(
        &app,
        request(
            Method::POST,
            "/api/v1/auth/refresh",
            None,
            Some(json!({ "refresh_token": second.refresh })),
        ),
    )
    .await;
    assert_eq!(refresh_status, StatusCode::UNAUTHORIZED);
    assert_eq!(sessions(&app, &first.access).await.len(), 1);
    let events = revocations(&database).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["session_ids"], json!([second_id]));
    assert_eq!(events[0]["target_user_ids"], json!([user_id]));
}

#[tokio::test]
async fn revoking_the_current_unknown_or_foreign_session_is_refused() {
    let database = start_database().await;
    let app = app(&database);
    let carol = register(&app, "carol").await;
    let mallory = register(&app, "mallory").await;
    let own = current_id(&sessions(&app, &carol.access).await);
    let foreign = current_id(&sessions(&app, &mallory.access).await);

    let (status, body) = delete_session(&app, &carol.access, &own).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["errors"]["session_id"]["_errors"][0]["code"],
        "CANNOT_REVOKE_CURRENT_SESSION"
    );
    for id in [
        foreign,
        uuid::Uuid::new_v4().to_string(),
        "not-a-uuid".to_owned(),
    ] {
        assert_eq!(
            delete_session(&app, &carol.access, &id).await.0,
            StatusCode::NOT_FOUND,
            "{id}"
        );
    }
    assert_eq!(status_of(&app, &mallory.access).await, StatusCode::OK);
    assert!(revocations(&database).await.is_empty());
}

#[tokio::test]
async fn logout_all_revokes_every_other_session_and_keeps_the_current_one() {
    let database = start_database().await;
    let app = app(&database);
    let first = register(&app, "dave").await;
    let second = login(&app, "dave", DESKTOP_UA, "NL").await;
    let third = login(&app, "dave", FIREFOX_UA, "DE").await;
    let mut expected: Vec<String> = sessions(&app, &third.access)
        .await
        .iter()
        .filter(|s| s["current"] == false)
        .map(|s| s["id"].as_str().expect("id").to_owned())
        .collect();
    expected.sort();

    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/users/@me/sessions/logout-all",
        &third.access,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        status_of(&app, &first.access).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status_of(&app, &second.access).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(status_of(&app, &third.access).await, StatusCode::OK);
    assert_eq!(sessions(&app, &third.access).await.len(), 1);
    let events = revocations(&database).await;
    assert_eq!(events.len(), 1);
    let mut published: Vec<String> = events[0]["data"]["session_ids"]
        .as_array()
        .expect("ids")
        .iter()
        .map(|id| id.as_str().expect("id").to_owned())
        .collect();
    published.sort();
    assert_eq!(published, expected);
}

#[tokio::test]
async fn refreshing_moves_last_used_forward() {
    let database = start_database().await;
    let app = app(&database);
    let session = register(&app, "erin").await;
    let before = sessions(&app, &session.access).await[0]["last_used_at"]
        .as_str()
        .expect("timestamp")
        .to_owned();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let (status, body) = send(
        &app,
        request(
            Method::POST,
            "/api/v1/auth/refresh",
            None,
            Some(json!({ "refresh_token": session.refresh })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let after = sessions(&app, body["access_token"].as_str().expect("token")).await[0]
        ["last_used_at"]
        .as_str()
        .expect("timestamp")
        .to_owned();
    assert!(after > before, "{after} should be after {before}");
}

#[tokio::test]
async fn refresh_reuse_announces_revocation_and_removes_push_tokens() {
    let database = start_database().await;
    let app = app(&database);
    let login = register(&app, "replay").await;
    let session_id = current_id(&sessions(&app, &login.access).await);
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/users/@me/devices/push",
        &login.access,
        Some(json!({"platform": "fcm", "token": "replay-test-device-token"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let refresh_request = || {
        request(
            Method::POST,
            "/api/v1/auth/refresh",
            None,
            Some(json!({"refresh_token": login.refresh})),
        )
    };
    let (status, rotated) = send(&app, refresh_request()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        send(&app, refresh_request()).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status_of(&app, rotated["access_token"].as_str().unwrap()).await,
        StatusCode::UNAUTHORIZED
    );
    let events = revocations(&database).await;
    assert_eq!(
        events.len(),
        1,
        "reuse must disconnect an already-connected gateway session"
    );
    assert_eq!(events[0]["data"]["session_ids"], json!([session_id]));
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM push_tokens")
        .fetch_one(&database.pool)
        .await
        .expect("push token count");
    assert_eq!(
        remaining, 0,
        "revoked sessions must stop receiving push notifications"
    );
}

#[tokio::test]
async fn logout_all_rolls_back_when_the_disconnect_event_cannot_be_saved() {
    let database = start_database().await;
    let app = app(&database);
    let first = register(&app, "atomic").await;
    let current = login(&app, "atomic", DESKTOP_UA, "NL").await;
    sqlx::query("ALTER TABLE outbox ADD CONSTRAINT reject_revocation CHECK (payload->>'type' <> 'SESSION_REVOKE')")
        .execute(&database.pool).await.expect("inject an outbox failure");
    let status = call(
        &app,
        Method::POST,
        "/api/v1/users/@me/sessions/logout-all",
        &current.access,
        None,
    )
    .await
    .0;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        status_of(&app, &first.access).await,
        StatusCode::OK,
        "revocation and its durable disconnect event must commit together"
    );
    sqlx::query("ALTER TABLE outbox DROP CONSTRAINT reject_revocation")
        .execute(&database.pool)
        .await
        .expect("restore outbox");
    assert_eq!(
        call(
            &app,
            Method::POST,
            "/api/v1/users/@me/sessions/logout-all",
            &current.access,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        revocations(&database).await.len(),
        1,
        "a retry must still disconnect the session"
    );
}

#[tokio::test]
async fn concurrent_refresh_cannot_revive_a_session_revoked_by_logout_all() {
    let database = start_database().await;
    let app = app(&database);
    let current = register(&app, "concurrent").await;
    for _ in 0..16 {
        let other = login(&app, "concurrent", FIREFOX_UA, "NL").await;
        let (refresh, logout) = tokio::join!(
            send(
                &app,
                request(
                    Method::POST,
                    "/api/v1/auth/refresh",
                    None,
                    Some(json!({"refresh_token": other.refresh}))
                )
            ),
            call(
                &app,
                Method::POST,
                "/api/v1/users/@me/sessions/logout-all",
                &current.access,
                None
            )
        );
        assert_eq!(logout.0, StatusCode::NO_CONTENT);
        assert!(matches!(
            refresh.0,
            StatusCode::OK | StatusCode::UNAUTHORIZED
        ));
        assert_eq!(
            status_of(&app, &other.access).await,
            StatusCode::UNAUTHORIZED
        );
        if refresh.0 == StatusCode::OK {
            assert_eq!(
                status_of(&app, refresh.1["access_token"].as_str().unwrap()).await,
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(status_of(&app, &current.access).await, StatusCode::OK);
    }
    assert_eq!(revocations(&database).await.len(), 16);
}
