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
async fn retrying_a_refresh_after_losing_the_response_recovers_the_same_successor() {
    let database = start_database().await;
    let app = app(&database);
    let saved = register(&app, "refreshrecovery").await;
    let body = json!({
        "refresh_token": saved.refresh,
        "request_id": "ae4b0d8e-06d1-4b72-9401-44f83fc966bf",
    });
    let (first_status, lost_response) = send(
        &app,
        request(
            Method::POST,
            "/api/v1/auth/refresh",
            None,
            Some(body.clone()),
        ),
    )
    .await;
    assert_eq!(first_status, StatusCode::OK);

    // The process died after the commit, before saving this response. A new
    // runtime knows only the old token and the request id saved before sending.
    let (retry_status, recovered) = send(
        &app,
        request(Method::POST, "/api/v1/auth/refresh", None, Some(body)),
    )
    .await;
    assert_eq!(
        retry_status,
        StatusCode::OK,
        "restart must recover the committed rotation"
    );
    assert!(
        recovered["refresh_token"] == lost_response["refresh_token"],
        "the retry returns the original successor"
    );
    assert_eq!(
        status_of(&app, &tokens(&recovered).access).await,
        StatusCode::OK
    );
    assert!(
        revocations(&database).await.is_empty(),
        "a legitimate retry must not revoke the session"
    );
}

async fn recoverable_refresh(
    app: &axum::Router,
    token: &str,
    id: uuid::Uuid,
) -> (StatusCode, Value) {
    send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/refresh",
            None,
            Some(json!({ "refresh_token": token, "request_id": id })),
        ),
    )
    .await
}

#[tokio::test]
async fn concurrent_identical_refresh_retries_converge_without_extending_expiry() {
    let database = start_database().await;
    let app = app(&database);
    let saved = register(&app, "samerequest").await;
    let id = uuid::Uuid::new_v4();
    let (a, b) = tokio::join!(
        recoverable_refresh(&app, &saved.refresh, id),
        recoverable_refresh(&app, &saved.refresh, id),
    );
    assert_eq!(a.0, StatusCode::OK);
    assert_eq!(b.0, StatusCode::OK);
    assert!(
        a.1["refresh_token"] == b.1["refresh_token"],
        "concurrent requests share one successor"
    );
    let expiries_before: Vec<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT expires_at FROM refresh_tokens ORDER BY token_hash")
            .fetch_all(&database.pool)
            .await
            .unwrap();
    for _ in 0..3 {
        assert_eq!(
            recoverable_refresh(&app, &saved.refresh, id).await.0,
            StatusCode::OK
        );
    }
    let expiries_after: Vec<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT expires_at FROM refresh_tokens ORDER BY token_hash")
            .fetch_all(&database.pool)
            .await
            .unwrap();
    assert_eq!(expiries_after, expiries_before);
    assert_eq!(expiries_after.len(), 2, "retries create no token branches");
    assert!(revocations(&database).await.is_empty());
}

#[tokio::test]
async fn a_different_refresh_request_id_still_revokes_the_family_and_push_tokens() {
    let database = start_database().await;
    let app = app(&database);
    let saved = register(&app, "differentrequest").await;
    let status = call(
        &app,
        Method::POST,
        "/api/v1/users/@me/devices/push",
        &saved.access,
        Some(json!({ "platform": "fcm", "token": "refresh-recovery-push-fixture" })),
    )
    .await
    .0;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, successor) = recoverable_refresh(&app, &saved.refresh, uuid::Uuid::new_v4()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        recoverable_refresh(&app, &saved.refresh, uuid::Uuid::new_v4())
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status_of(&app, &tokens(&successor).access).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(revocations(&database).await.len(), 1);
    let pushes: i64 = sqlx::query_scalar("SELECT count(*) FROM push_tokens")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    assert_eq!(pushes, 0);
}

#[tokio::test]
async fn recovery_refuses_expired_or_used_successors_and_elapsed_windows() {
    let database = start_database().await;
    let app = app(&database);
    for scenario in ["expired", "used", "window", "parentexpired", "future"] {
        let saved = register(&app, scenario).await;
        let id = uuid::Uuid::new_v4();
        let (status, response) = recoverable_refresh(&app, &saved.refresh, id).await;
        assert_eq!(status, StatusCode::OK);
        let successor = tokens(&response);
        let session_id = current_id(&sessions(&app, &successor.access).await);
        let session_id = uuid::Uuid::parse_str(&session_id).unwrap();
        match scenario {
            "expired" => {
                sqlx::query("UPDATE refresh_tokens SET expires_at = now() - interval '1 second' WHERE session_id = $1 AND rotated_at IS NULL")
                    .bind(session_id).execute(&database.pool).await.unwrap();
            }
            "used" => {
                assert_eq!(
                    recoverable_refresh(&app, &successor.refresh, uuid::Uuid::new_v4())
                        .await
                        .0,
                    StatusCode::OK
                );
            }
            "window" => {
                sqlx::query("UPDATE refresh_tokens SET rotated_at = now() - interval '6 minutes' WHERE session_id = $1 AND rotated_at IS NOT NULL")
                    .bind(session_id).execute(&database.pool).await.unwrap();
            }
            "parentexpired" => {
                sqlx::query("UPDATE refresh_tokens SET expires_at = now() - interval '1 second' WHERE session_id = $1 AND rotated_at IS NOT NULL")
                    .bind(session_id).execute(&database.pool).await.unwrap();
            }
            _ => {
                sqlx::query("UPDATE refresh_tokens SET rotated_at = now() + interval '1 minute' WHERE session_id = $1 AND rotated_at IS NOT NULL")
                    .bind(session_id).execute(&database.pool).await.unwrap();
            }
        }
        assert_eq!(
            recoverable_refresh(&app, &saved.refresh, id).await.0,
            StatusCode::UNAUTHORIZED,
            "{scenario}"
        );
        assert_eq!(
            status_of(&app, &successor.access).await,
            StatusCode::UNAUTHORIZED,
            "{scenario}"
        );
    }
    assert_eq!(revocations(&database).await.len(), 5);
}

#[tokio::test]
async fn malformed_recovery_ids_do_not_rotate_and_omitted_ids_keep_strict_reuse_detection() {
    let database = start_database().await;
    let app = app(&database);
    let saved = register(&app, "requestvalidation").await;
    let (status, _) = send(
        &app,
        request(
            Method::POST,
            "/api/v1/auth/refresh",
            None,
            Some(json!({ "refresh_token": saved.refresh, "request_id": "not-a-uuid" })),
        ),
    )
    .await;
    assert!(status.is_client_error());
    let unrotated: i64 =
        sqlx::query_scalar("SELECT count(*) FROM refresh_tokens WHERE rotated_at IS NULL")
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(unrotated, 1);
    let (status, successor) = recoverable_refresh(&app, &saved.refresh, uuid::Uuid::new_v4()).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(
        &app,
        request(
            Method::POST,
            "/api/v1/auth/refresh",
            None,
            Some(json!({ "refresh_token": saved.refresh })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        status_of(&app, &tokens(&successor).access).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(revocations(&database).await.len(), 1);
}

#[tokio::test]
async fn recovery_cannot_restore_a_logged_out_remotely_revoked_or_password_changed_session() {
    let database = start_database().await;
    let app = app(&database);
    for scenario in ["logout", "remote", "password"] {
        let saved = register(&app, scenario).await;
        let other = login(&app, scenario, FIREFOX_UA, "NL").await;
        let id = uuid::Uuid::new_v4();
        let (status, response) = recoverable_refresh(&app, &saved.refresh, id).await;
        assert_eq!(status, StatusCode::OK);
        let successor = tokens(&response);
        let session_id = current_id(&sessions(&app, &successor.access).await);
        let status =
            match scenario {
                "logout" => {
                    call(
                        &app,
                        Method::POST,
                        "/api/v1/auth/logout",
                        &successor.access,
                        None,
                    )
                    .await
                    .0
                }
                "remote" => delete_session(&app, &other.access, &session_id).await.0,
                _ => call(
                    &app,
                    Method::PATCH,
                    "/api/v1/users/@me",
                    &other.access,
                    Some(
                        json!({ "password": PASSWORD, "new_password": "the-new-long-passphrase" }),
                    ),
                )
                .await
                .0,
            };
        assert!(status.is_success(), "{scenario}: {status}");
        assert_eq!(
            recoverable_refresh(&app, &saved.refresh, id).await.0,
            StatusCode::UNAUTHORIZED,
            "{scenario}"
        );
        assert_eq!(
            status_of(&app, &successor.access).await,
            StatusCode::UNAUTHORIZED,
            "{scenario}"
        );
    }
}

#[tokio::test]
async fn refresh_retry_and_logout_cannot_commit_a_live_session_after_revocation() {
    let database = start_database().await;
    let app = app(&database);
    for index in 0..8 {
        let saved = register(&app, &format!("retryrace{index}")).await;
        let id = uuid::Uuid::new_v4();
        let (status, response) = recoverable_refresh(&app, &saved.refresh, id).await;
        assert_eq!(status, StatusCode::OK);
        let successor = tokens(&response);
        let (retry, logout) = tokio::join!(
            recoverable_refresh(&app, &saved.refresh, id),
            call(
                &app,
                Method::POST,
                "/api/v1/auth/logout",
                &successor.access,
                None
            ),
        );
        assert!(retry.0 == StatusCode::OK || retry.0 == StatusCode::UNAUTHORIZED);
        assert_eq!(logout.0, StatusCode::NO_CONTENT);
        assert_eq!(
            status_of(&app, &successor.access).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            recoverable_refresh(&app, &saved.refresh, id).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(revocations(&database).await.len(), 8);
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
