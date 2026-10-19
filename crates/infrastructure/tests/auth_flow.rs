//! Auth flows end to end against Postgres: register, login, refresh rotation,
//! reuse detection, logout and bot introspection.

mod support;

use application::use_cases::auth::{authenticate, login, logout, refresh, register};
use application::use_cases::internal::introspect;
use application::AppError;

use support::{jwt_keys, ports, start_database};

fn registration(email: &str, username: &str) -> register::RegisterInput {
    register::RegisterInput {
        email: email.to_owned(),
        username: username.to_owned(),
        password: "a-long-enough-passphrase".to_owned(),
    }
}

#[tokio::test]
async fn auth_lifecycle_rotates_and_revokes_sessions() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let ports = ports(&database.pool, &public, &private);

    let registered = register::execute(&ports, registration("Alice@Example.com", "alice"))
        .await
        .expect("register");
    assert_eq!(registered.user.username, "alice");

    let duplicate = register::execute(&ports, registration("alice@example.com", "other")).await;
    assert!(matches!(duplicate, Err(AppError::Validation(_))));

    let wrong_password = login::execute(
        &ports,
        login::LoginInput {
            email: "alice@example.com".into(),
            password: "not-the-password".into(),
            totp: None,
            device_id: None,
        },
    )
    .await;
    assert!(matches!(wrong_password, Err(AppError::Validation(_))));

    let session = login::execute(
        &ports,
        login::LoginInput {
            email: "alice@example.com".into(),
            password: "a-long-enough-passphrase".into(),
            totp: None,
            device_id: None,
        },
    )
    .await
    .expect("login");

    let actor = authenticate::execute(
        &ports,
        authenticate::Credential::Bearer(&session.access_token),
    )
    .await
    .expect("access token authenticates");
    assert_eq!(actor.user_id, session.user.id);

    let rotated = refresh::execute(&ports, &session.refresh_token)
        .await
        .expect("first refresh");
    assert_ne!(rotated.refresh_token, session.refresh_token);

    let reused = refresh::execute(&ports, &session.refresh_token).await;
    assert!(
        matches!(reused, Err(AppError::Unauthorized)),
        "reuse must be rejected"
    );
    let family_revoked = refresh::execute(&ports, &rotated.refresh_token).await;
    assert!(
        matches!(family_revoked, Err(AppError::Unauthorized)),
        "reuse detection revokes the whole session family"
    );

    let fresh = login::execute(
        &ports,
        login::LoginInput {
            email: "alice@example.com".into(),
            password: "a-long-enough-passphrase".into(),
            totp: None,
            device_id: None,
        },
    )
    .await
    .expect("login again");
    logout::execute(
        &ports,
        &authenticate::execute(
            &ports,
            authenticate::Credential::Bearer(&fresh.access_token),
        )
        .await
        .expect("authenticate"),
    )
    .await
    .expect("logout");
    let after_logout = refresh::execute(&ports, &fresh.refresh_token).await;
    assert!(matches!(after_logout, Err(AppError::Unauthorized)));
}

#[tokio::test]
async fn introspection_rejects_unknown_tokens_and_reports_user_tokens() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let ports = ports(&database.pool, &public, &private);
    let session = register::execute(&ports, registration("bob@example.com", "bob"))
        .await
        .expect("register");

    let introspected = introspect::execute(&ports, &session.access_token)
        .await
        .expect("user jwt introspects");
    assert_eq!(introspected.user_id, session.user.id);
    assert!(introspected.session_id.is_some());

    let forged = introspect::execute(&ports, "Bot not-a-real-token").await;
    assert!(forged.is_err());
}

#[tokio::test]
async fn totp_enrolment_gates_password_login() {
    use application::ports::TotpVerifier;
    use application::use_cases::auth::{authenticate, enable_totp};
    use infrastructure::security::totp::Rfc6238;

    let database = start_database().await;
    let (private, public) = jwt_keys();
    let ports = ports(&database.pool, &public, &private);
    let session = register::execute(&ports, registration("carol@example.com", "carol"))
        .await
        .expect("register");
    let actor = authenticate::execute(
        &ports,
        authenticate::Credential::Bearer(&session.access_token),
    )
    .await
    .expect("authenticate");

    let secret = Rfc6238.generate_secret();
    let code = totp_code(&secret);
    enable_totp::execute(
        &ports,
        &actor,
        enable_totp::EnableTotpInput {
            secret: secret.clone(),
            code: code.clone(),
            password: "a-long-enough-passphrase".into(),
        },
    )
    .await
    .expect("enrol totp");

    let without_code = login::execute(
        &ports,
        login::LoginInput {
            email: "carol@example.com".into(),
            password: "a-long-enough-passphrase".into(),
            totp: None,
            device_id: None,
        },
    )
    .await;
    assert!(matches!(without_code, Err(AppError::MfaRequired)));

    let wrong_code = login::execute(
        &ports,
        login::LoginInput {
            email: "carol@example.com".into(),
            password: "a-long-enough-passphrase".into(),
            totp: Some("000000".into()),
            device_id: None,
        },
    )
    .await;
    assert!(wrong_code.is_err());

    let with_code = login::execute(
        &ports,
        login::LoginInput {
            email: "carol@example.com".into(),
            password: "a-long-enough-passphrase".into(),
            totp: Some(code),
            device_id: None,
        },
    )
    .await;
    assert!(with_code.is_ok(), "a valid TOTP code completes the login");
}

fn totp_code(secret: &str) -> String {
    use totp_rs::{Algorithm, Secret, TOTP};
    let bytes = Secret::Encoded(secret.to_owned())
        .to_bytes()
        .expect("base32 secret");
    let totp = TOTP::new(Algorithm::SHA1, 6, 1, 30, bytes).expect("totp");
    totp.generate_current().expect("clock")
}
