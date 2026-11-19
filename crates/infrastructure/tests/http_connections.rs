//! End-to-end OAuth callback against an in-process provider, not live GitHub.
mod support;
use axum::{
    extract::{Form, Query, State},
    http::{Method, StatusCode},
    response::Redirect,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use interface::state::{AppState, RatePolicy};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use support::http::{call, request, send};

#[derive(Deserialize)]
struct Authorize {
    state: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
}
async fn authorize(
    State(challenge): State<Arc<Mutex<String>>>,
    Query(q): Query<Authorize>,
) -> Redirect {
    assert_eq!(q.code_challenge_method, "S256");
    *challenge.lock().unwrap() = q.code_challenge;
    let mut url = url::Url::parse(&q.redirect_uri).unwrap();
    url.query_pairs_mut()
        .append_pair("state", &q.state)
        .append_pair("code", "test-code");
    Redirect::temporary(url.as_str())
}
#[derive(Deserialize)]
struct TokenRequest {
    code: String,
    code_verifier: String,
    client_id: String,
    client_secret: String,
}
async fn token(
    State(challenge): State<Arc<Mutex<String>>>,
    Form(body): Form<TokenRequest>,
) -> Json<serde_json::Value> {
    assert_eq!(body.code, "test-code");
    assert_eq!(body.client_id, "test-id");
    assert_eq!(body.client_secret, "test-secret");
    assert_eq!(
        URL_SAFE_NO_PAD.encode(Sha256::digest(body.code_verifier.as_bytes())),
        *challenge.lock().unwrap()
    );
    Json(json!({"access_token":"test-token"}))
}
async fn user(headers: axum::http::HeaderMap) -> Json<serde_json::Value> {
    assert_eq!(headers["authorization"], "Bearer test-token");
    Json(json!({"id":123, "login":"test-user"}))
}
#[tokio::test]
async fn github_flow_visibility_and_replay() {
    let db = support::start_database().await;
    let provider_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = format!("http://{}", provider_listener.local_addr().unwrap());
    let routes = Router::new()
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .route("/user", get(user))
        .with_state(Arc::new(Mutex::new(String::new())));
    let provider_task =
        tokio::spawn(async move { axum::serve(provider_listener, routes).await.unwrap() });
    let api_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", api_listener.local_addr().unwrap());
    let (private, public) = support::jwt_keys();
    let mut ports = support::ports(&db.pool, &public, &private);
    ports.connections = Some(Arc::new(
        infrastructure::connections::GithubConnections::with_endpoints(
            db.pool.clone(),
            "test-id".into(),
            "test-secret".into(),
            format!("{base}/api/v1/connections/github/callback")
                .parse()
                .unwrap(),
            format!("{provider}/authorize").parse().unwrap(),
            format!("{provider}/token").parse().unwrap(),
            format!("{provider}/user").parse().unwrap(),
        )
        .unwrap(),
    ));
    let purge_ports = ports.clone();
    let app = interface::router(AppState {
        ports,
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: Arc::new(support::AllowAll),
        rate_policy: RatePolicy::default(),
    });
    let (status, registration) = send(&app, request(Method::POST, "/api/v1/auth/register", None, Some(json!({"email":"alice@example.com","username":"alice","password":"a-long-enough-passphrase"})))).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, second) = send(&app, request(Method::POST, "/api/v1/auth/register", None, Some(json!({"email":"bob@example.com","username":"bob","password":"a-long-enough-passphrase"})))).await;
    assert_eq!(status, StatusCode::CREATED);
    let second_access = second["access_token"].as_str().unwrap();
    let access = registration["access_token"].as_str().unwrap();
    let id = registration["user"]["id"].as_str().unwrap();
    let (status, start) = call(
        &app,
        Method::POST,
        "/api/v1/users/@me/connections/github/authorize",
        access,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let api_task = tokio::spawn(async move { axum::serve(api_listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = client
        .get(start["url"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    let callback = response.headers()["location"].to_str().unwrap().to_owned();
    let completed = client.get(&callback).send().await.unwrap();
    assert_eq!(completed.status(), StatusCode::OK);
    assert_eq!(completed.headers()["cache-control"], "no-store");
    assert_eq!(completed.headers()["referrer-policy"], "no-referrer");
    assert_eq!(
        client.get(&callback).send().await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    // A provider identity already linked to another user produces a conflict.
    let second_start: serde_json::Value = client
        .post(format!(
            "{base}/api/v1/users/@me/connections/github/authorize"
        ))
        .bearer_auth(second_access)
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let second_redirect = client
        .get(second_start["url"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    let second_callback = second_redirect.headers()["location"]
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        client.get(second_callback).send().await.unwrap().status(),
        StatusCode::CONFLICT
    );
    let bearer = |url: String| client.get(url).bearer_auth(access);
    let mine: serde_json::Value = bearer(format!("{base}/api/v1/users/@me/connections"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(mine[0]["name"], "test-user");
    assert_eq!(mine[0]["visible"], false);
    let public: serde_json::Value = bearer(format!("{base}/api/v1/users/{id}/connections"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(public, json!([]));
    assert_eq!(
        client
            .put(format!("{base}/api/v1/users/@me/connections"))
            .bearer_auth(access)
            .json(&json!({"provider":"github", "visible":true}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    let public: serde_json::Value = bearer(format!("{base}/api/v1/users/{id}/connections"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(public[0]["name"], "test-user");
    // Disconnect invalidates an in-flight authorization as well as the existing link.
    let pending: serde_json::Value = client
        .post(format!(
            "{base}/api/v1/users/@me/connections/github/authorize"
        ))
        .bearer_auth(access)
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let redirect = client
        .get(pending["url"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    let pending_callback = redirect.headers()["location"].to_str().unwrap().to_owned();
    assert_eq!(
        client
            .delete(format!("{base}/api/v1/users/@me/connections"))
            .bearer_auth(access)
            .json(&json!({"provider":"github"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        client.get(pending_callback).send().await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    let remaining: serde_json::Value = bearer(format!("{base}/api/v1/users/@me/connections"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(remaining, json!([]));
    // Re-link and create another pending state, then verify anonymization removes both.
    let restart: serde_json::Value = client
        .post(format!(
            "{base}/api/v1/users/@me/connections/github/authorize"
        ))
        .bearer_auth(access)
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let redirect = client
        .get(restart["url"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    let callback = redirect.headers()["location"].to_str().unwrap().to_owned();
    assert_eq!(
        client.get(callback).send().await.unwrap().status(),
        StatusCode::OK
    );
    let pending: serde_json::Value = client
        .post(format!(
            "{base}/api/v1/users/@me/connections/github/authorize"
        ))
        .bearer_auth(access)
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(pending["url"].is_string());
    let mut writes = application::write_set::WriteSet::new();
    writes.mutate(application::write_set::Mutation::AnonymizeUser {
        user_id: id.parse().unwrap(),
        username: "deleted-alice".into(),
    });
    purge_ports.unit_of_work.commit(writes).await.unwrap();
    let stored: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM user_connections WHERE user_id=$1), (SELECT count(*) FROM connection_oauth_states WHERE user_id=$1)")
        .bind(id.parse::<i64>().unwrap()).fetch_one(&db.pool).await.unwrap();
    assert_eq!(stored, (0, 0));
    api_task.abort();
    provider_task.abort();
}
