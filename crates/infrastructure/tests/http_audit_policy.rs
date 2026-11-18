mod support;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};
use support::http::{call, request, send};

async fn fixture() -> (support::Database, axum::Router, String, String) {
    let db = support::start_database().await;
    let (private, public) = support::jwt_keys();
    let app = interface::router(interface::state::AppState {
        ports: support::ports(&db.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let (status, user) = send(&app, request(Method::POST, "/api/v1/auth/register", None,
        Some(json!({"email":"owner@example.com","username":"owner","password":"a-long-enough-passphrase"})))).await;
    assert_eq!(status, StatusCode::CREATED, "{user}");
    let token = user["access_token"].as_str().unwrap().to_owned();
    let (_, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &token,
        Some(json!({"name":"Community"})),
    )
    .await;
    (db, app, token, guild["id"].as_str().unwrap().into())
}

async fn entries(app: &axum::Router, token: &str, guild: &str) -> Vec<Value> {
    let (status, body) = call(
        app,
        Method::GET,
        &format!("/api/v1/guilds/{guild}/audit-logs"),
        token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["audit_log_entries"].as_array().unwrap().clone()
}

#[tokio::test]
async fn disabling_audit_preserves_existing_entries_and_suppresses_new_entries_and_dispatches() {
    let (db, app, token, guild) = fixture().await;
    let path = format!("/api/v1/guilds/{guild}");
    let (_, initial) = call(&app, Method::GET, &path, &token, None).await;
    assert_eq!(initial["audit_log_enabled"], true);
    assert_eq!(initial["audit_log_retention_days"], 90);
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            &token,
            Some(json!({"name":"Before"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let before = entries(&app, &token, &guild).await.len();
    let (status, body) = call(
        &app,
        Method::PATCH,
        &path,
        &token,
        Some(json!({"audit_log_enabled":false,"audit_log_retention_days":30})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["audit_log_enabled"], false);
    assert_eq!(body["audit_log_retention_days"], 30);
    assert_eq!(entries(&app, &token, &guild).await.len(), before + 1);
    let outbox_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox WHERE payload::text LIKE '%AUDIT_LOG_ENTRY_CREATE%'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(
        outbox_before > 0,
        "enabled and policy-change audits are dispatched"
    );
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            &token,
            Some(json!({"name":"While disabled"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            Method::POST,
            &format!("{path}/channels"),
            &token,
            Some(json!({"name":"private-work","type":0}))
        )
        .await
        .0,
        StatusCode::CREATED
    );
    assert_eq!(entries(&app, &token, &guild).await.len(), before + 1);
    let outbox_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox WHERE payload::text LIKE '%AUDIT_LOG_ENTRY_CREATE%'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        outbox_before, outbox_after,
        "suppressed audits must not enter the event stream"
    );
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            &token,
            Some(json!({"audit_log_enabled":true}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(entries(&app, &token, &guild).await.len(), before + 2);
}

#[tokio::test]
async fn only_administrators_change_policy_and_retention_is_validated() {
    let (db, app, token, guild) = fixture().await;
    let path = format!("/api/v1/guilds/{guild}");
    for days in [0, 7, 31, 366] {
        assert_eq!(
            call(
                &app,
                Method::PATCH,
                &path,
                &token,
                Some(json!({"audit_log_retention_days":days}))
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let (_, guest) = send(&app, request(Method::POST, "/api/v1/auth/register", None,
        Some(json!({"email":"manager@example.com","username":"manager","password":"a-long-enough-passphrase"})))).await;
    let guest_id: i64 = guest["user"]["id"].as_str().unwrap().parse().unwrap();
    let guild_id: i64 = guild.parse().unwrap();
    sqlx::query("INSERT INTO members (guild_id,user_id,joined_at) VALUES ($1,$2,now())")
        .bind(guild_id)
        .bind(guest_id)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE roles SET permissions = permissions | 32 | 128 WHERE id = $1")
        .bind(guild_id)
        .execute(&db.pool)
        .await
        .unwrap();
    let guest_token = guest["access_token"].as_str().unwrap();
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            guest_token,
            Some(json!({"name":"Managers may rename"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            guest_token,
            Some(json!({"audit_log_enabled":false}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            guest_token,
            Some(json!({"audit_log_retention_days":365}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert!(
        !entries(&app, guest_token, &guild).await.is_empty(),
        "VIEW_AUDIT_LOG remains usable per role"
    );
}

#[tokio::test]
async fn retention_excludes_and_prunes_old_entries_even_when_logging_is_disabled() {
    use application::ports::AuditLogRepository;
    let (db, app, token, guild) = fixture().await;
    let path = format!("/api/v1/guilds/{guild}");
    let guild_id: i64 = guild.parse().unwrap();
    for kind in ["AUDIT_LOG_ENTRY_CREATE", "GUILD_UPDATE"] {
        sqlx::query("INSERT INTO outbox (event_id,topic,key,payload,created_at,sent_at) VALUES ($1,'discordy.audit.v1',$2,$3,now() - interval '31 days',now())")
            .bind(uuid::Uuid::now_v7()).bind(&guild)
            .bind(json!({"type":kind,"guild_id":guild}))
            .execute(&db.pool).await.unwrap();
    }
    sqlx::query(
        "INSERT INTO audit_log (id,guild_id,action,changes,created_at)
        VALUES (1,$1,1,'[]',now() - interval '31 days'),
               (2,$1,1,'[]',now() - interval '29 days')",
    )
    .bind(guild_id)
    .execute(&db.pool)
    .await
    .unwrap();
    let (status, body) = call(
        &app,
        Method::PATCH,
        &path,
        &token,
        Some(json!({"audit_log_enabled":false,"audit_log_retention_days":30})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed = entries(&app, &token, &guild).await;
    assert!(listed.iter().any(|e| e["id"] == "2"));
    assert!(!listed.iter().any(|e| e["id"] == "1"));
    let store = infrastructure::postgres::PgStore::new(db.pool.clone());
    assert_eq!(store.prune_expired(chrono::Utc::now()).await.unwrap(), 1);
    assert_eq!(store.prune_expired(chrono::Utc::now()).await.unwrap(), 0);
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM outbox WHERE created_at < now() - interval '30 days' AND payload->>'type' = 'AUDIT_LOG_ENTRY_CREATE'")
        .fetch_one(&db.pool).await.unwrap(), 0, "expired event copies must not remain in the outbox");
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM outbox WHERE created_at < now() - interval '30 days' AND payload->>'type' = 'GUILD_UPDATE'")
        .fetch_one(&db.pool).await.unwrap(), 1, "other events remain available");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM audit_log WHERE id = 1")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        0
    );
}
