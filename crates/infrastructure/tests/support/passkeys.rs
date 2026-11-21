//! A software authenticator creates signed assertions; no verification mocks.
#![allow(dead_code)]
use super::http::{call, request, send, INTERNAL_TOKEN};
use axum::{
    http::{Method, StatusCode},
    Router,
};
use serde_json::{json, Value};
use webauthn_authenticator_rs::{softpasskey::SoftPasskey, WebauthnAuthenticator};

pub const PASSWORD: &str = "a-long-enough-passphrase";
pub const ORIGIN: &str = "https://localhost:8443";
pub const REGISTER_START: &str = "/api/v1/users/@me/mfa/passkeys/register/start";
pub const REGISTER_FINISH: &str = "/api/v1/users/@me/mfa/passkeys/register/finish";
pub const AUTH_START: &str = "/api/v1/auth/mfa/webauthn/start";
pub const AUTH_FINISH: &str = "/api/v1/auth/mfa/webauthn/finish";
pub type Authenticator = WebauthnAuthenticator<SoftPasskey>;

pub struct Fixture {
    pub database: super::Database,
    pub app: Router,
    pub replica: Router,
    pub ports: application::Ports,
    pub access: String,
    pub user_id: domain::Snowflake,
}

pub fn app(ports: application::Ports) -> Router {
    interface::router(interface::state::AppState {
        ports,
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(super::AllowAll),
        rate_policy: Default::default(),
    })
}

pub async fn fixture() -> Fixture {
    let database = super::start_database().await;
    let (private, public) = super::jwt_keys();
    let ports = super::ports(&database.pool, &public, &private);
    // Separate application/verifier adapters share only PostgreSQL.
    let replica = app(super::ports(&database.pool, &public, &private));
    let app = app(ports.clone());
    let (status, user) = public_post(
        &app,
        "/api/v1/auth/register",
        json!({"username":"passkey", "email":"passkey@example.com", "password":PASSWORD}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{user}");
    Fixture {
        database,
        app,
        replica,
        ports,
        access: user["access_token"].as_str().unwrap().into(),
        user_id: domain::Snowflake::new(user["user"]["id"].as_str().unwrap().parse().unwrap()),
    }
}

pub async fn public_post(app: &Router, path: &str, value: Value) -> (StatusCode, Value) {
    send(app, request(Method::POST, path, None, Some(value))).await
}

pub async fn login(app: &Router) -> (StatusCode, Value) {
    public_post(
        app,
        "/api/v1/auth/login",
        json!({"email":"passkey@example.com", "password":PASSWORD}),
    )
    .await
}

pub async fn ticket(app: &Router) -> String {
    let (status, body) = login(app).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["code"], 60008);
    body["ticket"].as_str().unwrap().into()
}

pub async fn registration(app: &Router, access: &str) -> Value {
    let (status, challenge) = call(
        app,
        Method::POST,
        REGISTER_START,
        access,
        Some(json!({"password":PASSWORD,"name":"Security key"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{challenge}");
    challenge
}

pub fn registration_response(
    authenticator: &mut Authenticator,
    challenge: &Value,
    origin: &str,
) -> Value {
    let options = serde_json::from_value(json!({"publicKey":challenge["publicKey"]})).unwrap();
    let credential = authenticator
        .do_registration(url::Url::parse(origin).unwrap(), options)
        .expect("software registration");
    json!({"challenge_id":challenge["challenge_id"], "credential":credential})
}

pub async fn enroll(f: &Fixture) -> (Authenticator, Value) {
    let mut authenticator = WebauthnAuthenticator::new(SoftPasskey::new(true));
    let challenge = registration(&f.app, &f.access).await;
    let body = registration_response(&mut authenticator, &challenge, ORIGIN);
    let (status, registered) =
        call(&f.app, Method::POST, REGISTER_FINISH, &f.access, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{registered}");
    (authenticator, registered)
}

pub async fn authentication(app: &Router, ticket: &str) -> Value {
    let (status, challenge) = public_post(app, AUTH_START, json!({"ticket":ticket})).await;
    assert_eq!(status, StatusCode::OK, "{challenge}");
    challenge
}

pub fn assertion(
    authenticator: &mut Authenticator,
    challenge: &Value,
    ticket: &str,
    origin: &str,
) -> Value {
    let options = serde_json::from_value(json!({"publicKey":challenge["publicKey"]})).unwrap();
    let credential = authenticator
        .do_authentication(url::Url::parse(origin).unwrap(), options)
        .expect("software authentication");
    json!({"ticket":ticket,"challenge_id":challenge["challenge_id"],"credential":credential})
}

pub fn recovery(registered: &Value, index: usize) -> &str {
    registered["backup_codes"][index]["code"].as_str().unwrap()
}

pub async fn me(f: &Fixture) -> Value {
    let (status, me) = call(&f.app, Method::GET, "/api/v1/users/@me", &f.access, None).await;
    assert_eq!(status, StatusCode::OK);
    me
}

pub fn totp_code(secret: &str) -> String {
    use totp_rs::{Algorithm, Secret, TOTP};
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        Secret::Encoded(secret.into()).to_bytes().unwrap(),
    )
    .unwrap()
    .generate_current()
    .unwrap()
}
