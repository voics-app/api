mod support;

use axum::http::{Method, StatusCode};
use serde_json::json;
use support::http::{call, request, send};

async fn register(app: &axum::Router, name: &str) -> (String, String) {
    let (status, body) = send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(
                json!({"email": format!("{name}@example.com"), "username": name,
            "password": "a-long-enough-passphrase"}),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    (
        body["access_token"].as_str().unwrap().into(),
        body["user"]["id"].as_str().unwrap().into(),
    )
}

async fn guild(app: &axum::Router, token: &str) -> String {
    let (status, body) = call(
        app,
        Method::POST,
        "/api/v1/guilds",
        token,
        Some(json!({"name": "Community"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().into()
}

fn app(pool: &sqlx::PgPool) -> axum::Router {
    let (private, public) = support::jwt_keys();
    interface::router(interface::state::AppState {
        ports: support::ports(pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    })
}

#[tokio::test]
async fn vanity_codes_resolve_join_and_revoke_without_bypassing_bans() {
    let db = support::start_database().await;
    let app = app(&db.pool);
    let (owner, _) = register(&app, "owner").await;
    let (guest, guest_id) = register(&app, "guest").await;
    let guild_id = guild(&app, &owner).await;
    let path = format!("/api/v1/guilds/{guild_id}/vanity-url");
    let (status, body) = call(&app, Method::GET, &path, &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({"code": null, "uses": 0}));
    let (status, body) = call(
        &app,
        Method::PATCH,
        &path,
        &owner,
        Some(json!({"code": "Study-Club"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({"code": "study-club", "uses": 0}));
    let (status, preview) = send(
        &app,
        request(Method::GET, "/api/v1/invites/STUDY-CLUB", None, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["guild"]["id"], guild_id);
    for _ in 0..2 {
        let (status, body) = call(
            &app,
            Method::POST,
            "/api/v1/invites/Study-Club",
            &guest,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert_eq!(
        call(&app, Method::GET, &path, &owner, None).await.1["uses"],
        1
    );
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            &guest,
            Some(json!({"code": "stolen"}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            Method::PUT,
            &format!("/api/v1/guilds/{guild_id}/bans/{guest_id}"),
            &owner,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let (status, body) = call(
        &app,
        Method::POST,
        "/api/v1/invites/study-club",
        &guest,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], 40007);
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            &owner,
            Some(json!({"code": "new-club"}))
        )
        .await
        .1["uses"],
        0
    );
    assert_eq!(
        send(
            &app,
            request(Method::GET, "/api/v1/invites/study-club", None, None)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            &owner,
            Some(json!({"code": null}))
        )
        .await
        .1,
        json!({"code":null,"uses":0})
    );
    assert_eq!(
        send(
            &app,
            request(Method::GET, "/api/v1/invites/new-club", None, None)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, log) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/audit-logs"),
        &owner,
        None,
    )
    .await;
    assert!(log["audit_log_entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["key"] == "vanity_url_code")));
}

#[tokio::test]
async fn vanity_claims_are_atomic_validate_codes_and_do_not_shadow_regular_invites() {
    let db = support::start_database().await;
    let app = app(&db.pool);
    let (owner, _) = register(&app, "owner").await;
    let one = guild(&app, &owner).await;
    let two = guild(&app, &owner).await;
    let first = format!("/api/v1/guilds/{one}/vanity-url");
    let second = format!("/api/v1/guilds/{two}/vanity-url");
    for body in [
        json!({}),
        json!({"code":""}),
        json!({"code":"x"}),
        json!({"code":"no space"}),
        json!({"code":"a".repeat(33)}),
    ] {
        assert_eq!(
            call(&app, Method::PATCH, &first, &owner, Some(body))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    let (a, b) = tokio::join!(
        call(
            &app,
            Method::PATCH,
            &first,
            &owner,
            Some(json!({"code":"Shared-Code"}))
        ),
        call(
            &app,
            Method::PATCH,
            &second,
            &owner,
            Some(json!({"code":"shared-code"}))
        )
    );
    assert!(
        matches!(
            (a.0, b.0),
            (StatusCode::OK, StatusCode::BAD_REQUEST) | (StatusCode::BAD_REQUEST, StatusCode::OK)
        ),
        "{a:?} {b:?}"
    );
    let (_, channels) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{one}/channels"),
        &owner,
        None,
    )
    .await;
    let channel = channels
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == 0)
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let (_, invite) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{channel}/invites"),
        &owner,
        Some(json!({})),
    )
    .await;
    let code = invite["code"].as_str().unwrap();
    let (status, body) = call(
        &app,
        Method::PATCH,
        &second,
        &owner,
        Some(json!({"code":code.to_uppercase()})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["errors"]["code"].is_object());
}

#[tokio::test]
async fn vanity_never_exposes_a_private_channel() {
    let db = support::start_database().await;
    let app = app(&db.pool);
    let (owner, _) = register(&app, "owner").await;
    let id = guild(&app, &owner).await;
    let path = format!("/api/v1/guilds/{id}/vanity-url");
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            &owner,
            Some(json!({"code":"private-test"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, body) = call(
        &app,
        Method::PATCH,
        &format!("/api/v1/guilds/{id}/roles/{id}"),
        &owner,
        Some(json!({"permissions":"0"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        send(
            &app,
            request(Method::GET, "/api/v1/invites/private-test", None, None)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            &owner,
            Some(json!({"code":"another-code"}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            &owner,
            Some(json!({"code":null}))
        )
        .await
        .1,
        json!({"code":null,"uses":0})
    );
}
