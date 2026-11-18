//! Recovery remains available for passkey-only accounts and coexists with TOTP.
mod support;
use axum::http::{Method, StatusCode};
use serde_json::json;
use support::http::call;
use support::passkeys::*;

#[tokio::test]
async fn recovery_codes_are_hashed_single_use_and_removal_requires_both_proofs() {
    let f = fixture().await;
    let (_, registered) = enroll(&f).await;
    let hashes: Vec<String> =
        sqlx::query_scalar("SELECT code_hash FROM user_backup_codes WHERE user_id = $1")
            .bind(f.user_id.as_i64())
            .fetch_all(&f.database.pool)
            .await
            .unwrap();
    assert_eq!(hashes.len(), 10);
    assert!(hashes.iter().all(|hash| hash.starts_with("$argon2id$")));
    assert!(hashes
        .iter()
        .all(|hash| !hash.contains(recovery(&registered, 0))));
    let ticket = ticket(&f.app).await;
    let body = json!({"ticket":ticket,"code":recovery(&registered, 0)});
    let (one, two) = tokio::join!(
        public_post(&f.app, "/api/v1/auth/mfa/totp", body.clone()),
        public_post(&f.replica, "/api/v1/auth/mfa/totp", body)
    );
    assert_eq!(
        [one.0, two.0]
            .iter()
            .filter(|s| **s == StatusCode::OK)
            .count(),
        1
    );
    let ticket = support::passkeys::ticket(&f.app).await;
    assert_eq!(
        public_post(
            &f.app,
            "/api/v1/auth/mfa/totp",
            json!({"ticket":ticket,"code":recovery(&registered,0)})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (_, listing) = call(
        &f.app,
        Method::POST,
        "/api/v1/users/@me/mfa/codes-verification",
        &f.access,
        Some(json!({"password":PASSWORD})),
    )
    .await;
    assert!(listing["backup_codes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|code| code.get("code").is_none()));
    let path = format!(
        "/api/v1/users/@me/mfa/passkeys/{}",
        registered["passkey"]["id"].as_str().unwrap()
    );
    for (password, code) in [("wrong", recovery(&registered, 1)), (PASSWORD, "wrong")] {
        assert_eq!(
            call(
                &f.app,
                Method::DELETE,
                &path,
                &f.access,
                Some(json!({"password":password,"code":code}))
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(me(&f).await["mfa_enabled"], true);
    assert_eq!(
        call(
            &f.app,
            Method::DELETE,
            &path,
            &f.access,
            Some(json!({"password":PASSWORD,"code":recovery(&registered,1)}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(me(&f).await["mfa_enabled"], false);
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM user_backup_codes WHERE user_id = $1")
            .bind(f.user_id.as_i64())
            .fetch_one(&f.database.pool)
            .await
            .unwrap();
    assert_eq!(remaining, 0);
    assert_eq!(login(&f.app).await.0, StatusCode::OK);
}

#[tokio::test]
async fn adding_and_removing_a_passkey_preserves_totp_and_existing_recovery_codes() {
    let f = fixture().await;
    let (_, pending) = call(
        &f.app,
        Method::POST,
        "/api/v1/users/@me/mfa/totp/enable",
        &f.access,
        Some(json!({"password":PASSWORD})),
    )
    .await;
    let secret = pending["secret"].as_str().unwrap();
    let (status, totp) = call(
        &f.app,
        Method::POST,
        "/api/v1/users/@me/mfa/totp/verify",
        &f.access,
        Some(json!({"code":totp_code(secret)})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, registered) = enroll(&f).await;
    assert!(
        registered.get("backup_codes").is_none(),
        "adding a passkey must not invalidate recovery codes"
    );
    let (_, challenge) = login(&f.app).await;
    assert_eq!(
        challenge["methods"],
        json!(["totp", "webauthn", "backup_code"])
    );
    let path = format!(
        "/api/v1/users/@me/mfa/passkeys/{}",
        registered["passkey"]["id"].as_str().unwrap()
    );
    assert_eq!(
        call(
            &f.app,
            Method::DELETE,
            &path,
            &f.access,
            Some(json!({"password":PASSWORD,"code":totp_code(secret)}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(me(&f).await["mfa_enabled"], true);
    let ticket = ticket(&f.app).await;
    assert_eq!(
        public_post(
            &f.app,
            "/api/v1/auth/mfa/totp",
            json!({"ticket":ticket,"code":totp["backup_codes"][0]["code"]})
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn removing_totp_keeps_passkeys_and_recovery_usable() {
    let f = fixture().await;
    let (_, pending) = call(
        &f.app,
        Method::POST,
        "/api/v1/users/@me/mfa/totp/enable",
        &f.access,
        Some(json!({"password":PASSWORD})),
    )
    .await;
    let secret = pending["secret"].as_str().unwrap();
    let (_, totp) = call(
        &f.app,
        Method::POST,
        "/api/v1/users/@me/mfa/totp/verify",
        &f.access,
        Some(json!({"code":totp_code(secret)})),
    )
    .await;
    let (mut authenticator, _) = enroll(&f).await;
    assert_eq!(
        call(
            &f.app,
            Method::POST,
            "/api/v1/users/@me/mfa/totp/disable",
            &f.access,
            Some(json!({"password":PASSWORD,"code":totp_code(secret)}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(me(&f).await["mfa_enabled"], true);
    let ticket = ticket(&f.app).await;
    let challenge = authentication(&f.app, &ticket).await;
    let body = assertion(&mut authenticator, &challenge, &ticket, ORIGIN);
    assert_eq!(
        public_post(&f.app, AUTH_FINISH, body).await.0,
        StatusCode::OK
    );
    let ticket = support::passkeys::ticket(&f.app).await;
    assert_eq!(
        public_post(
            &f.app,
            "/api/v1/auth/mfa/totp",
            json!({"ticket":ticket,"code":totp["backup_codes"][0]["code"]})
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn unavailable_webauthn_does_not_bypass_mfa_and_recovery_can_be_regenerated() {
    let f = fixture().await;
    let (_, registered) = enroll(&f).await;
    let mut ports = f.ports.clone();
    ports.webauthn = None;
    let app = app(ports);
    assert_eq!(
        call(
            &app,
            Method::POST,
            REGISTER_START,
            &f.access,
            Some(json!({"password":PASSWORD,"name":"key"}))
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let (status, challenge) = login(&app).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(challenge["methods"], json!(["backup_code"]));
    let (status, codes) = call(
        &app,
        Method::POST,
        "/api/v1/users/@me/mfa/codes-verification",
        &f.access,
        Some(json!({"password":PASSWORD,"regenerate":true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        public_post(
            &app,
            "/api/v1/auth/mfa/totp",
            json!({"ticket":challenge["ticket"],"code":recovery(&registered,0)})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        public_post(
            &app,
            "/api/v1/auth/mfa/totp",
            json!({"ticket":challenge["ticket"],"code":codes["backup_codes"][0]["code"]})
        )
        .await
        .0,
        StatusCode::OK
    );
}
