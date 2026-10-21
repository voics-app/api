//! Account management and two-factor over the real router: verification,
//! password reset and change, email change, TOTP enrolment, backup codes,
//! lockout, disable and delete.

mod support;

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use application::use_cases::account::purge_deleted;
use application::Ports;
use interface::state::{AppState, RatePolicy};
use support::http::{call, request, send};
use support::{jwt_keys, ports, start_database, BudgetLimiter, RecordingMail};

const PASSWORD: &str = "a-long-enough-passphrase";

fn app_with(ports: Ports) -> axum::Router {
    interface::router(AppState {
        ports,
        internal_token: "test-internal-token".into(),
        rate_limiter: Arc::new(support::AllowAll),
        rate_policy: RatePolicy::default(),
    })
}

async fn register(app: &axum::Router, email: &str, username: &str) -> Value {
    let (status, body) = send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({"email": email, "username": username, "password": PASSWORD})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}

async fn login(app: &axum::Router, email: &str, password: &str) -> (StatusCode, Value) {
    send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/login",
            None,
            Some(json!({"email": email, "password": password})),
        ),
    )
    .await
}

async fn post(
    app: &axum::Router,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    send(app, request(Method::POST, path, token, Some(body))).await
}

fn totp_code(secret: &str) -> String {
    use totp_rs::{Algorithm, Secret, TOTP};
    let bytes = Secret::Encoded(secret.to_owned())
        .to_bytes()
        .expect("base32 secret");
    TOTP::new(Algorithm::SHA1, 6, 1, 30, bytes)
        .expect("totp")
        .generate_current()
        .expect("clock")
}

async fn setup() -> (support::Database, Ports, Arc<RecordingMail>) {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let mut ports = ports(&database.pool, &public, &private);
    let mail = Arc::new(RecordingMail::default());
    ports.mailer = mail.clone();
    (database, ports, mail)
}

#[tokio::test]
async fn verification_and_password_recovery_by_email() {
    let (_db, ports, mail) = setup().await;
    let app = app_with(ports);
    let user = register(&app, "dana@example.com", "dana").await;
    let token = user["access_token"].as_str().unwrap().to_owned();
    assert_eq!(user["user"]["verified"], false);

    let code = mail
        .latest_token("dana@example.com")
        .expect("verification mail");
    let (status, _) = post(
        &app,
        "/api/v1/auth/verify-email",
        None,
        json!({"token": "nope"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = post(
        &app,
        "/api/v1/auth/verify-email",
        None,
        json!({"token": code}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, me) = call(&app, Method::GET, "/api/v1/users/@me", &token, None).await;
    assert_eq!(me["verified"], true);
    let (status, _) = post(
        &app,
        "/api/v1/auth/verify-email",
        None,
        json!({"token": code}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "tokens are single use");
    let (status, _) = post(
        &app,
        "/api/v1/auth/resend-verification",
        Some(&token),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "already verified");

    // Unknown addresses look the same and send nothing.
    let before = mail.sent.lock().unwrap().len();
    let (status, _) = post(
        &app,
        "/api/v1/auth/forgot-password",
        None,
        json!({"email": "ghost@example.com"}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(mail.sent.lock().unwrap().len(), before);

    let (status, _) = post(
        &app,
        "/api/v1/auth/forgot-password",
        None,
        json!({"email": "Dana@Example.com"}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let reset = mail.latest_token("dana@example.com").expect("reset mail");
    let (status, _) = post(
        &app,
        "/api/v1/auth/reset-password",
        None,
        json!({"token": reset, "password": "short"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a rejected password does not burn the token"
    );
    let (status, _) = post(
        &app,
        "/api/v1/auth/reset-password",
        None,
        json!({"token": reset, "password": "a-brand-new-passphrase"}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        login(&app, "dana@example.com", PASSWORD).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        login(&app, "dana@example.com", "a-brand-new-passphrase")
            .await
            .0,
        StatusCode::OK
    );
    let (status, _) = call(&app, Method::GET, "/api/v1/users/@me", &token, None).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a reset signs every session out"
    );
    let (status, _) = post(
        &app,
        "/api/v1/auth/reset-password",
        None,
        json!({"token": reset, "password": "yet-another-passphrase"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "reset tokens are single use"
    );
}

#[tokio::test]
async fn sensitive_profile_changes_need_the_password() {
    let (_db, ports, mail) = setup().await;
    let app = app_with(ports);
    let first = register(&app, "erin@example.com", "erin").await;
    let token = first["access_token"].as_str().unwrap().to_owned();
    let other = login(&app, "erin@example.com", PASSWORD).await.1;
    let other_refresh = other["refresh_token"].as_str().unwrap().to_owned();

    let patch = |body: Value| {
        let app = app.clone();
        let token = token.clone();
        async move { call(&app, Method::PATCH, "/api/v1/users/@me", &token, Some(body)).await }
    };
    let (status, _) = patch(json!({"new_password": "another-long-passphrase"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "password is required");
    let (status, body) =
        patch(json!({"username": "erin2", "password": "wrong-password-here"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["errors"]["password"].is_object());
    let (status, body) = patch(json!({"username": "erin2", "password": PASSWORD})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["username"], "erin2");
    let (status, body) = patch(json!({"global_name": "Erin"})).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "profile edits need no password: {body}"
    );

    // Email changes only take effect once the new address is verified.
    let (status, body) =
        patch(json!({"email": "erin.new@example.com", "password": PASSWORD})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["email"], "erin@example.com");
    let code = mail
        .latest_token("erin.new@example.com")
        .expect("mail to the new address");
    let (status, _) = post(
        &app,
        "/api/v1/auth/verify-email",
        None,
        json!({"token": code}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, me) = call(&app, Method::GET, "/api/v1/users/@me", &token, None).await;
    assert_eq!(me["email"], "erin.new@example.com");
    assert_eq!(
        login(&app, "erin.new@example.com", PASSWORD).await.0,
        StatusCode::OK
    );

    // A password change signs the other sessions out and keeps this one.
    let (status, body) =
        patch(json!({"new_password": "another-long-passphrase", "password": PASSWORD})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = call(&app, Method::GET, "/api/v1/users/@me", &token, None).await;
    assert_eq!(status, StatusCode::OK, "the calling session survives");
    let (status, _) = post(
        &app,
        "/api/v1/auth/refresh",
        None,
        json!({"refresh_token": other_refresh}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "other sessions are revoked"
    );
    assert_eq!(
        login(&app, "erin.new@example.com", "another-long-passphrase")
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn totp_enrolment_login_step_and_backup_codes() {
    let (_db, ports, _mail) = setup().await;
    let app = app_with(ports);
    let user = register(&app, "finn@example.com", "finn").await;
    let token = user["access_token"].as_str().unwrap().to_owned();

    let (status, _) = post(
        &app,
        "/api/v1/users/@me/mfa/totp/enable",
        Some(&token),
        json!({"password": "wrong-password-here"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, enrolment) = post(
        &app,
        "/api/v1/users/@me/mfa/totp/enable",
        Some(&token),
        json!({"password": PASSWORD}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{enrolment}");
    let secret = enrolment["secret"].as_str().unwrap().to_owned();
    assert!(enrolment["otpauth_uri"]
        .as_str()
        .unwrap()
        .starts_with("otpauth://totp/Discordy:"));
    let (_, me) = call(&app, Method::GET, "/api/v1/users/@me", &token, None).await;
    assert_eq!(me["mfa_enabled"], false, "not active before verification");

    let (status, _) = post(
        &app,
        "/api/v1/users/@me/mfa/totp/verify",
        Some(&token),
        json!({"code": "000000"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, codes) = post(
        &app,
        "/api/v1/users/@me/mfa/totp/verify",
        Some(&token),
        json!({"code": totp_code(&secret)}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{codes}");
    let backup: Vec<String> = codes["backup_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["code"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(backup.len(), 10);
    let (_, me) = call(&app, Method::GET, "/api/v1/users/@me", &token, None).await;
    assert_eq!(me["mfa_enabled"], true);

    // The password alone now yields a ticket, not a session.
    let (status, challenge) = login(&app, "finn@example.com", PASSWORD).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(challenge["code"], 60008);
    let ticket = challenge["ticket"].as_str().unwrap().to_owned();
    let (status, _) = post(
        &app,
        "/api/v1/auth/mfa/totp",
        None,
        json!({"ticket": ticket, "code": "000000"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, session) = post(
        &app,
        "/api/v1/auth/mfa/totp",
        None,
        json!({"ticket": ticket, "code": totp_code(&secret)}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let (status, _) = post(
        &app,
        "/api/v1/auth/mfa/totp",
        None,
        json!({"ticket": ticket, "code": totp_code(&secret)}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a ticket completes one login"
    );

    // A backup code works once, in either spelling.
    let (_, challenge) = login(&app, "finn@example.com", PASSWORD).await;
    let ticket = challenge["ticket"].as_str().unwrap().to_owned();
    let (status, _) = post(
        &app,
        "/api/v1/auth/mfa/totp",
        None,
        json!({"ticket": ticket, "code": backup[0].to_uppercase()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = post(
        &app,
        "/api/v1/auth/login",
        None,
        json!({"email": "finn@example.com", "password": PASSWORD, "totp": backup[0]}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "reused backup code: {body}"
    );

    // Listing shows consumed state only; regenerating replaces every code.
    let (status, listing) = post(
        &app,
        "/api/v1/users/@me/mfa/codes-verification",
        Some(&token),
        json!({"password": PASSWORD}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entries = listing["backup_codes"].as_array().unwrap();
    assert!(entries.iter().all(|c| c.get("code").is_none()));
    assert_eq!(entries.iter().filter(|c| c["consumed"] == true).count(), 1);
    let (status, fresh) = post(
        &app,
        "/api/v1/users/@me/mfa/codes-verification",
        Some(&token),
        json!({"password": PASSWORD, "regenerate": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let new_first = fresh["backup_codes"][0]["code"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, _) = post(
        &app,
        "/api/v1/auth/login",
        None,
        json!({"email": "finn@example.com", "password": PASSWORD, "totp": backup[1]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "old codes died");
    let (status, _) = post(
        &app,
        "/api/v1/auth/login",
        None,
        json!({"email": "finn@example.com", "password": PASSWORD, "totp": new_first}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Disabling needs the password and a code, and clears the secret.
    let (status, _) = post(
        &app,
        "/api/v1/users/@me/mfa/totp/disable",
        Some(&token),
        json!({"password": PASSWORD, "code": "000000"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = post(
        &app,
        "/api/v1/users/@me/mfa/totp/disable",
        Some(&token),
        json!({"password": PASSWORD, "code": totp_code(&secret)}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, me) = call(&app, Method::GET, "/api/v1/users/@me", &token, None).await;
    assert_eq!(me["mfa_enabled"], false);
    assert_eq!(
        login(&app, "finn@example.com", PASSWORD).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn repeated_wrong_codes_lock_the_second_factor() {
    let (_db, mut ports, _mail) = setup().await;
    let app = app_with(ports.clone());
    let user = register(&app, "gus@example.com", "gus").await;
    let token = user["access_token"].as_str().unwrap().to_owned();
    let (_, enrolment) = post(
        &app,
        "/api/v1/users/@me/mfa/totp/enable",
        Some(&token),
        json!({"password": PASSWORD}),
    )
    .await;
    let secret = enrolment["secret"].as_str().unwrap().to_owned();
    post(
        &app,
        "/api/v1/users/@me/mfa/totp/verify",
        Some(&token),
        json!({"code": totp_code(&secret)}),
    )
    .await;

    ports.limiter = Arc::new(BudgetLimiter::new(3));
    let app = app_with(ports);
    let mut statuses = Vec::new();
    for _ in 0..5 {
        let (status, body) = post(
            &app,
            "/api/v1/auth/login",
            None,
            json!({"email": "gus@example.com", "password": PASSWORD, "totp": "000000"}),
        )
        .await;
        statuses.push((status, body["retry_after"].is_number()));
    }
    assert_eq!(statuses[0].0, StatusCode::BAD_REQUEST);
    assert_eq!(statuses[4], (StatusCode::TOO_MANY_REQUESTS, true));
    let (status, _) = post(
        &app,
        "/api/v1/auth/login",
        None,
        json!({"email": "gus@example.com", "password": PASSWORD, "totp": totp_code(&secret)}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "even the right code is refused while locked"
    );
}

#[tokio::test]
async fn disable_and_delete_sign_out_and_login_reactivates() {
    let (db, ports, _mail) = setup().await;
    let app = app_with(ports.clone());
    let user = register(&app, "hana@example.com", "hana").await;
    let token = user["access_token"].as_str().unwrap().to_owned();
    let refresh = user["refresh_token"].as_str().unwrap().to_owned();

    let (status, _) = post(
        &app,
        "/api/v1/users/@me/disable",
        Some(&token),
        json!({"password": "wrong-password-here"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = post(
        &app,
        "/api/v1/users/@me/disable",
        Some(&token),
        json!({"password": PASSWORD}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = post(
        &app,
        "/api/v1/auth/refresh",
        None,
        json!({"refresh_token": refresh}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, back) = login(&app, "hana@example.com", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "logging in reactivates");
    let token = back["access_token"].as_str().unwrap().to_owned();
    let state = ports
        .users
        .account_state(back["user"]["id"].as_str().unwrap().parse().unwrap())
        .await
        .unwrap();
    assert!(!state.is_inactive());

    // Deletion is scheduled, cancelled by logging in, and final after the window.
    let (status, _) = post(
        &app,
        "/api/v1/users/@me/delete",
        Some(&token),
        json!({"password": PASSWORD}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, back) = login(&app, "hana@example.com", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let token = back["access_token"].as_str().unwrap().to_owned();
    assert_eq!(purge_deleted::execute(&ports).await.unwrap(), 0);

    let (status, _) = post(
        &app,
        "/api/v1/users/@me/delete",
        Some(&token),
        json!({"password": PASSWORD}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    sqlx::query(
        "UPDATE users SET delete_after = now() - interval '1 minute' WHERE username = 'hana'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    assert_eq!(purge_deleted::execute(&ports).await.unwrap(), 1);
    assert_eq!(
        login(&app, "hana@example.com", PASSWORD).await.0,
        StatusCode::BAD_REQUEST
    );
    let (username, email): (String, Option<String>) =
        sqlx::query_as("SELECT username, email FROM users WHERE username LIKE 'deleted_%'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(username.starts_with("deleted_") && email.is_none());
}

#[tokio::test]
async fn owners_must_hand_over_guilds_before_deleting() {
    let (_db, ports, _mail) = setup().await;
    let app = app_with(ports);
    let user = register(&app, "ivy@example.com", "ivy").await;
    let token = user["access_token"].as_str().unwrap().to_owned();
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &token,
        Some(json!({"name": "Ivy HQ"})),
    )
    .await;
    assert!(status.is_success());
    let (status, body) = post(
        &app,
        "/api/v1/users/@me/delete",
        Some(&token),
        json!({"password": PASSWORD}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], 20011);
}
