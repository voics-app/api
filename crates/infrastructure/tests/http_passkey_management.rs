//! Passkey labels never change credentials or another account's keys.
mod support;

use axum::http::{Method, StatusCode};
use serde_json::json;
use support::http::{call, request, send};
use support::passkeys::*;

#[tokio::test]
async fn rename_is_owner_bound_validated_and_does_not_change_mfa_or_credentials() {
    let f = fixture().await;
    let (_, registered) = enroll(&f).await;
    let path = format!(
        "/api/v1/users/@me/mfa/passkeys/{}",
        registered["passkey"]["id"].as_str().unwrap()
    );
    let before: (serde_json::Value, i64) = sqlx::query_as(
        "SELECT credential, signature_counter FROM user_passkeys WHERE user_id = $1",
    )
    .bind(f.user_id.as_i64())
    .fetch_one(&f.database.pool)
    .await
    .unwrap();
    let (status, renamed) = call(
        &f.app,
        Method::PATCH,
        &path,
        &f.access,
        Some(json!({"name":"  Laptop key 🔑  "})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["name"], "Laptop key 🔑");
    let created = |value: &serde_json::Value| {
        chrono::DateTime::parse_from_rfc3339(value.as_str().unwrap())
            .unwrap()
            .timestamp_micros()
    };
    assert_eq!(
        created(&renamed["created_at"]),
        created(&registered["passkey"]["created_at"])
    );
    assert_eq!(renamed.as_object().unwrap().len(), 4);
    assert_eq!(me(&f).await["mfa_enabled"], true);
    assert_eq!(
        send(
            &f.app,
            request(Method::PATCH, &path, None, Some(json!({"name":"No"})))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    for name in [" ".into(), "x".repeat(65), "key\u{0000}".into()] {
        assert_eq!(
            call(
                &f.app,
                Method::PATCH,
                &path,
                &f.access,
                Some(json!({"name":name}))
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let (_, other) = public_post(
        &f.app,
        "/api/v1/auth/register",
        json!({"username":"other", "email":"other@example.com", "password":PASSWORD}),
    )
    .await;
    let access = other["access_token"].as_str().unwrap();
    let foreign = call(
        &f.app,
        Method::PATCH,
        &path,
        access,
        Some(json!({"name":"Stolen"})),
    )
    .await;
    let missing = call(
        &f.app,
        Method::PATCH,
        &format!("/api/v1/users/@me/mfa/passkeys/{}", uuid::Uuid::new_v4()),
        access,
        Some(json!({"name":"Missing"})),
    )
    .await;
    assert_eq!(foreign, missing);
    assert_eq!(foreign.0, StatusCode::BAD_REQUEST);
    let after: (serde_json::Value, i64) = sqlx::query_as(
        "SELECT credential, signature_counter FROM user_passkeys WHERE user_id = $1",
    )
    .bind(f.user_id.as_i64())
    .fetch_one(&f.database.pool)
    .await
    .unwrap();
    assert_eq!(before, after);
    let (_, keys) = call(
        &f.app,
        Method::GET,
        "/api/v1/users/@me/mfa/passkeys",
        &f.access,
        None,
    )
    .await;
    assert_eq!(keys[0]["name"], "Laptop key 🔑");
    let ticket = ticket(&f.app).await;
    assert_eq!(
        public_post(
            &f.app,
            "/api/v1/auth/mfa/totp",
            json!({"ticket":ticket,"code":recovery(&registered, 0)})
        )
        .await
        .0,
        StatusCode::OK
    );
}
