//! Passkeys through real HTTP, PostgreSQL and a cryptographic software authenticator.
mod support;

use axum::http::{Method, StatusCode};
use serde_json::json;
use support::http::call;
use support::passkeys::*;
use webauthn_authenticator_rs::{softpasskey::SoftPasskey, WebauthnAuthenticator};

#[tokio::test]
async fn authenticated_registration_creates_a_challenge_without_enabling_mfa() {
    let f = fixture().await;
    let challenge = registration(&f.app, &f.access).await;
    assert!(challenge["challenge_id"].is_string());
    assert!(challenge["publicKey"]["challenge"].is_string());
    assert_eq!(
        challenge["publicKey"]["authenticatorSelection"]["userVerification"],
        "required"
    );
    assert_eq!(me(&f).await["mfa_enabled"], false);
    assert_eq!(
        public_post(
            &f.app,
            REGISTER_START,
            json!({"password":PASSWORD,"name":"Key"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    for (password, name) in [("wrong", "Key"), (PASSWORD, "   ")] {
        let (status, _) = call(
            &f.app,
            Method::POST,
            REGISTER_START,
            &f.access,
            Some(json!({"password":password,"name":name})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn passkey_only_account_requires_a_second_factor_and_accepts_signed_login() {
    let f = fixture().await;
    let (mut authenticator, registered) = enroll(&f).await;
    assert_eq!(registered["backup_codes"].as_array().unwrap().len(), 10);
    assert_eq!(me(&f).await["mfa_enabled"], true);
    let (_, error) = login(&f.app).await;
    assert_eq!(error["methods"], json!(["webauthn", "backup_code"]));
    let ticket = error["ticket"].as_str().unwrap();
    let challenge = authentication(&f.app, ticket).await;
    assert_eq!(challenge["publicKey"]["userVerification"], "required");
    let body = assertion(&mut authenticator, &challenge, ticket, ORIGIN);
    let (status, session) = public_post(&f.replica, AUTH_FINISH, body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{session}");
    assert!(session["access_token"].is_string());
    assert_eq!(session["user"]["id"], me(&f).await["id"]);
    assert_eq!(
        public_post(&f.app, AUTH_FINISH, body).await.0,
        StatusCode::BAD_REQUEST
    );
    let (_, keys) = call(
        &f.app,
        Method::GET,
        "/api/v1/users/@me/mfa/passkeys",
        &f.access,
        None,
    )
    .await;
    assert_eq!(keys[0]["id"], registered["passkey"]["id"]);
    assert!(keys[0]["last_used_at"].is_string());
    assert_eq!(
        keys[0].as_object().unwrap().len(),
        4,
        "credential material must never be serialized"
    );
}

#[tokio::test]
async fn registration_is_single_use_and_bound_to_session_expiry_and_exact_origin() {
    let f = fixture().await;
    let (_, second_login) = login(&f.app).await;
    let second_access = second_login["access_token"].as_str().unwrap();
    let mut authenticator = WebauthnAuthenticator::new(SoftPasskey::new(true));
    let challenge = registration(&f.app, &f.access).await;
    let wrong = registration_response(&mut authenticator, &challenge, "https://localhost:9443");
    let (status, _) = call(
        &f.app,
        Method::POST,
        REGISTER_FINISH,
        &f.access,
        Some(wrong),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "another port is not trusted"
    );
    let valid = registration_response(&mut authenticator, &challenge, ORIGIN);
    let (status, _) = call(
        &f.app,
        Method::POST,
        REGISTER_FINISH,
        &f.access,
        Some(valid),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "failed finish consumes its challenge"
    );
    let expired = registration(&f.app, &f.access).await;
    sqlx::query("UPDATE passkey_challenges SET expires_at = now() - interval '1 second'")
        .execute(&f.database.pool)
        .await
        .unwrap();
    let body = registration_response(&mut authenticator, &expired, ORIGIN);
    assert_eq!(
        call(&f.app, Method::POST, REGISTER_FINISH, &f.access, Some(body))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(me(&f).await["mfa_enabled"], false);
    let challenge = registration(&f.app, &f.access).await;
    let body = registration_response(&mut authenticator, &challenge, ORIGIN);
    assert_eq!(
        call(
            &f.app,
            Method::POST,
            REGISTER_FINISH,
            second_access,
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, registered) = call(
        &f.replica,
        Method::POST,
        REGISTER_FINISH,
        &f.access,
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{registered}");
    assert_eq!(
        call(&f.app, Method::POST, REGISTER_FINISH, &f.access, Some(body))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn authentication_rejects_wrong_origin_signature_ticket_and_expired_challenge() {
    let f = fixture().await;
    let (mut authenticator, _) = enroll(&f).await;
    let ticket = ticket(&f.app).await;
    let challenge = authentication(&f.app, &ticket).await;
    let wrong = assertion(
        &mut authenticator,
        &challenge,
        &ticket,
        "https://localhost:9443",
    );
    assert_eq!(
        public_post(&f.app, AUTH_FINISH, wrong).await.0,
        StatusCode::BAD_REQUEST
    );
    let challenge = authentication(&f.app, &ticket).await;
    let mut forged = assertion(&mut authenticator, &challenge, &ticket, ORIGIN);
    forged["credential"]["response"]["signature"] = json!("AAAA");
    assert_eq!(
        public_post(&f.app, AUTH_FINISH, forged).await.0,
        StatusCode::BAD_REQUEST
    );
    let challenge = authentication(&f.app, &ticket).await;
    let valid = assertion(&mut authenticator, &challenge, &ticket, ORIGIN);
    let mut wrong_ticket = valid.clone();
    wrong_ticket["ticket"] = json!("not-a-password-proof");
    assert_eq!(
        public_post(&f.app, AUTH_FINISH, wrong_ticket).await.0,
        StatusCode::BAD_REQUEST
    );
    sqlx::query("UPDATE passkey_challenges SET expires_at = now() - interval '1 second'")
        .execute(&f.database.pool)
        .await
        .unwrap();
    assert_eq!(
        public_post(&f.app, AUTH_FINISH, valid).await.0,
        StatusCode::BAD_REQUEST
    );
    let challenge = authentication(&f.app, &ticket).await;
    let valid = assertion(&mut authenticator, &challenge, &ticket, ORIGIN);
    assert_eq!(
        public_post(&f.app, AUTH_FINISH, valid).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn concurrent_replicas_can_consume_a_challenge_and_ticket_only_once() {
    let f = fixture().await;
    let (mut authenticator, _) = enroll(&f).await;
    let first_ticket = ticket(&f.app).await;
    let challenge = authentication(&f.app, &first_ticket).await;
    let body = assertion(&mut authenticator, &challenge, &first_ticket, ORIGIN);
    let (one, two) = tokio::join!(
        public_post(&f.app, AUTH_FINISH, body.clone()),
        public_post(&f.replica, AUTH_FINISH, body)
    );
    assert_eq!(
        [one.0, two.0]
            .iter()
            .filter(|s| **s == StatusCode::OK)
            .count(),
        1,
        "{one:?} {two:?}"
    );
    let ticket = ticket(&f.app).await;
    let challenge_a = authentication(&f.app, &ticket).await;
    let challenge_b = authentication(&f.replica, &ticket).await;
    let body_a = assertion(&mut authenticator, &challenge_a, &ticket, ORIGIN);
    let body_b = assertion(&mut authenticator, &challenge_b, &ticket, ORIGIN);
    let (one, two) = tokio::join!(
        public_post(&f.app, AUTH_FINISH, body_a),
        public_post(&f.replica, AUTH_FINISH, body_b)
    );
    assert_eq!(
        [one.0, two.0]
            .iter()
            .filter(|s| **s == StatusCode::OK)
            .count(),
        1,
        "distinct challenges share one ticket: {one:?} {two:?}"
    );
    let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE user_id = $1")
        .bind(f.user_id.as_i64())
        .fetch_one(&f.database.pool)
        .await
        .unwrap();
    assert_eq!(
        sessions, 3,
        "original session and exactly two completed MFA logins"
    );
}

#[tokio::test]
async fn stale_signature_counter_is_rejected_without_consuming_its_login_ticket() {
    let f = fixture().await;
    let (mut authenticator, registered) = enroll(&f).await;
    let ticket_a = ticket(&f.app).await;
    let ticket_b = ticket(&f.app).await;
    let a = authentication(&f.app, &ticket_a).await;
    let b = authentication(&f.app, &ticket_b).await;
    let body_a = assertion(&mut authenticator, &a, &ticket_a, ORIGIN);
    let body_b = assertion(&mut authenticator, &b, &ticket_b, ORIGIN);
    assert_eq!(
        public_post(&f.replica, AUTH_FINISH, body_b).await.0,
        StatusCode::OK
    );
    assert_eq!(
        public_post(&f.app, AUTH_FINISH, body_a).await.0,
        StatusCode::BAD_REQUEST
    );
    let (status, _) = public_post(
        &f.app,
        "/api/v1/auth/mfa/totp",
        json!({"ticket":ticket_a, "code":recovery(&registered, 0)}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "recovery remains available after a rejected counter"
    );
}
