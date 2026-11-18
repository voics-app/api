//! Real PostgreSQL HTTP regressions for webhook credentials and bot consents.
#[path = "http_integrations/lifecycle.rs"]
mod lifecycle;
mod support;
#[path = "http_integrations/webhooks.rs"]
mod webhooks;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use application::ports::UnitOfWork;
use application::write_set::{Mutation, WriteSet};
use application::{AppResult, Ports};
use async_trait::async_trait;
use axum::http::{Method, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tokio::sync::Notify;

use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database};

fn router(mut ports: Ports) -> Router {
    ports.messages = Arc::new(support::RecordingMessages::default());
    interface::router(AppState {
        ports,
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    })
}

async fn register(app: &Router, name: &str) -> String {
    let (status, body) = send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({
                "email": format!("{name}@example.test"), "username": name,
                "password": "a-long-enough-passphrase"
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    body["access_token"]
        .as_str()
        .expect("access token")
        .to_owned()
}

async fn guild(app: &Router, owner: &str) -> (String, String) {
    let (status, body) = call(
        app,
        Method::POST,
        "/api/v1/guilds",
        owner,
        Some(json!({"name": "Study club"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["id"].as_str().expect("guild id").to_owned();
    let (status, channels) = call(
        app,
        Method::GET,
        &format!("/api/v1/guilds/{id}/channels"),
        owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let channel = channels
        .as_array()
        .expect("channels")
        .iter()
        .find(|c| c["type"] == 0)
        .expect("text channel")["id"]
        .as_str()
        .expect("channel id")
        .to_owned();
    (id, channel)
}

async fn webhook(app: &Router, owner: &str, channel: &str) -> (String, String) {
    let (status, body) = call(
        app,
        Method::POST,
        &format!("/api/v1/channels/{channel}/webhooks"),
        owner,
        Some(json!({"name": "Reminders"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    (
        body["id"].as_str().expect("webhook id").to_owned(),
        body["token"].as_str().expect("execution token").to_owned(),
    )
}

async fn execute_hook(app: &Router, id: &str, token: &str) -> StatusCode {
    send(
        app,
        request(
            Method::POST,
            &format!("/api/v1/webhooks/{id}/{token}"),
            None,
            Some(json!({"content": "Reminder"})),
        ),
    )
    .await
    .0
}

async fn application(app: &Router, owner: &str) -> (String, String) {
    let (status, body) = call(
        app,
        Method::POST,
        "/api/v1/applications",
        owner,
        Some(json!({"name": "Study helper"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    (
        body["id"].as_str().expect("application id").to_owned(),
        body["bot"]["id"].as_str().expect("bot id").to_owned(),
    )
}

async fn authorize(
    app: &Router,
    user: &str,
    application: &str,
    guild: &str,
    permissions: &str,
) -> (StatusCode, Value) {
    let preview = authorization_preview(app, user, application, guild, permissions).await;
    authorize_revision(
        app,
        user,
        application,
        guild,
        permissions,
        preview["installation_revision"].as_str().unwrap(),
    )
    .await
}

async fn authorization_preview(
    app: &Router,
    user: &str,
    application: &str,
    guild: &str,
    permissions: &str,
) -> Value {
    let path = format!("/api/v1/oauth2/authorize?client_id={application}&scope=bot+applications.commands&permissions={permissions}");
    let (status, info) = call(app, Method::GET, &path, user, None).await;
    assert_eq!(status, StatusCode::OK);
    info["guilds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["id"] == guild)
        .expect("eligible guild")
        .clone()
}

async fn authorize_revision(
    app: &Router,
    user: &str,
    application: &str,
    guild: &str,
    permissions: &str,
    revision: &str,
) -> (StatusCode, Value) {
    call(app, Method::POST, "/api/v1/oauth2/authorize", user, Some(json!({"client_id": application, "guild_id": guild, "permissions": permissions, "scope": "bot applications.commands", "installation_revision": revision}))).await
}

async fn reauthorize_current(
    app: &Router,
    user: &str,
    application: &str,
    guild: &str,
) -> (StatusCode, Value) {
    let preview = authorization_preview(app, user, application, guild, "0").await;
    authorize_revision(
        app,
        user,
        application,
        guild,
        preview["bot_permissions"].as_str().unwrap(),
        preview["installation_revision"].as_str().unwrap(),
    )
    .await
}

#[tokio::test]
async fn personal_consents_are_distinct_from_owned_apps_and_shared_installations() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = router(ports(&database.pool, &public, &private));
    let owner = register(&app, "manager").await;
    let outsider = register(&app, "outsider").await;
    let (guild, _) = guild(&app, &owner).await;
    let (application, bot) = application(&app, &owner).await;
    let grants_path = "/api/v1/oauth2/tokens";
    let installations_path = format!("/api/v1/guilds/{guild}/integrations");
    let (status, empty) = call(&app, Method::GET, grants_path, &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(empty, json!([]), "owning an application is not consent");
    let (status, response) = authorize(&app, &owner, &application, &guild, "2048").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["location"], format!("/channels/{guild}"));
    let (status, grants) = call(&app, Method::GET, grants_path, &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(grants.as_array().map(Vec::len), Some(1));
    assert_eq!(grants[0]["application"]["id"], application);
    assert_eq!(grants[0]["guild"]["id"], guild);
    assert_eq!(grants[0]["scopes"], json!(["bot", "applications.commands"]));
    let grant = grants[0]["id"].as_str().expect("consent id").to_owned();
    assert!(grants[0].get("token").is_none());
    let installed = call(&app, Method::GET, &installations_path, &owner, None).await;
    assert_eq!(installed.0, StatusCode::OK);
    assert_eq!(installed.1.as_array().map(Vec::len), Some(1));
    assert_eq!(grants[0]["permissions"], installed.1[0]["permissions"]);
    assert_eq!(
        call(&app, Method::GET, grants_path, &outsider, None)
            .await
            .1,
        json!([])
    );
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("{grants_path}/{grant}"),
            &outsider,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, Method::GET, &installations_path, &outsider, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    // Re-consenting must acknowledge current permissions, not escalate an installed bot.
    assert_eq!(
        authorize(&app, &owner, &application, &guild, "8").await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        reauthorize_current(&app, &owner, &application, &guild)
            .await
            .0,
        StatusCode::OK
    );
    let repeated = call(&app, Method::GET, grants_path, &owner, None).await.1;
    assert_eq!(repeated.as_array().map(Vec::len), Some(1));
    assert_eq!(repeated[0]["id"], grant);
    assert_eq!(repeated[0]["permissions"], grants[0]["permissions"]);
    let roles: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM roles WHERE guild_id=$1 AND bot_id=$2 AND managed",
    )
    .bind(guild.parse::<i64>().unwrap())
    .bind(bot.parse::<i64>().unwrap())
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(roles, 1);
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("{grants_path}/{grant}"),
            &owner,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, Method::GET, grants_path, &owner, None).await.1,
        json!([])
    );
    assert_eq!(
        call(
            &app,
            Method::GET,
            &format!("/api/v1/guilds/{guild}/members/{bot}"),
            &owner,
            None
        )
        .await
        .0,
        StatusCode::OK,
        "personal revocation leaves the shared installation"
    );

    assert_eq!(
        reauthorize_current(&app, &owner, &application, &guild)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("{installations_path}/{application}"),
            &owner,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, Method::GET, grants_path, &owner, None).await.1,
        json!([])
    );
    assert_eq!(
        call(&app, Method::GET, &installations_path, &owner, None)
            .await
            .1,
        json!([])
    );
    let roles: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM roles WHERE guild_id=$1 AND bot_id=$2 AND managed",
    )
    .bind(guild.parse::<i64>().unwrap())
    .bind(bot.parse::<i64>().unwrap())
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(roles, 0);
    assert_eq!(
        authorize(&app, &owner, &application, &guild, "2048")
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/v1/guilds/{guild}/members/{bot}"),
            &owner,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, Method::GET, grants_path, &owner, None).await.1,
        json!([]),
        "ordinary kick also clears consent"
    );
    assert_eq!(
        call(&app, Method::GET, &installations_path, &owner, None)
            .await
            .1,
        json!([])
    );
}

/// Pauses exactly one matching commit, after the real route has read its snapshot.
struct PauseCommit {
    inner: Arc<dyn UnitOfWork>,
    matches: fn(&Mutation) -> bool,
    armed: AtomicBool,
    reached: Notify,
    release: Notify,
}

#[async_trait]
impl UnitOfWork for PauseCommit {
    async fn commit(&self, writes: WriteSet) -> AppResult<()> {
        if writes.mutations.iter().any(self.matches) && self.armed.swap(false, Ordering::SeqCst) {
            self.reached.notify_one();
            self.release.notified().await;
        }
        self.inner.commit(writes).await
    }
}

#[tokio::test]
async fn webhook_rotation_invalidates_the_previous_execution_credential() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = router(ports(&database.pool, &public, &private));
    let owner = register(&app, "manager").await;
    let (_, channel) = guild(&app, &owner).await;
    let (id, old) = webhook(&app, &owner, &channel).await;
    let (status, rotated) = call(
        &app,
        Method::POST,
        &format!("/api/v1/webhooks/{id}/reset-token"),
        &owner,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "authenticated credential rotation succeeds"
    );
    let new = rotated["token"].as_str().expect("new token");
    assert!(
        old.as_str() != new,
        "rotation creates a different credential"
    );
    assert_eq!(execute_hook(&app, &id, &old).await, StatusCode::NOT_FOUND);
    assert_eq!(execute_hook(&app, &id, new).await, StatusCode::OK);
}

#[tokio::test]
async fn a_metadata_edit_cannot_restore_a_rotated_token_or_recreate_a_deleted_hook() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let mut ports = ports(&database.pool, &public, &private);
    let paused = Arc::new(PauseCommit {
        inner: ports.unit_of_work.clone(),
        matches: |m| matches!(m, Mutation::UpdateWebhook { .. }),
        armed: AtomicBool::new(false),
        reached: Notify::new(),
        release: Notify::new(),
    });
    ports.unit_of_work = paused.clone();
    let app = router(ports);
    let owner = register(&app, "manager").await;
    let (_, channel) = guild(&app, &owner).await;
    let (id, old) = webhook(&app, &owner, &channel).await;
    paused.armed.store(true, Ordering::SeqCst);
    let editing = {
        let app = app.clone();
        let owner = owner.clone();
        let id = id.clone();
        tokio::spawn(async move {
            call(
                &app,
                Method::PATCH,
                &format!("/api/v1/webhooks/{id}"),
                &owner,
                Some(json!({"name": "Announcements"})),
            )
            .await
        })
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        paused.reached.notified(),
    )
    .await
    .expect("edit reached commit");
    let (status, rotated) = call(
        &app,
        Method::POST,
        &format!("/api/v1/webhooks/{id}/reset-token"),
        &owner,
        None,
    )
    .await;
    paused.release.notify_one();
    let (edit_status, edited) = editing.await.expect("metadata task");
    assert_eq!(status, StatusCode::OK);
    assert_eq!(edit_status, StatusCode::OK);
    assert_eq!(edited["name"], "Announcements");
    assert!(edited.get("token").is_none());
    assert_eq!(
        execute_hook(&app, &id, &old).await,
        StatusCode::NOT_FOUND,
        "retired token must stay retired after the stale metadata commit"
    );
    assert_eq!(
        execute_hook(&app, &id, rotated["token"].as_str().expect("new token")).await,
        StatusCode::OK
    );

    paused.armed.store(true, Ordering::SeqCst);
    let editing = {
        let app = app.clone();
        let owner = owner.clone();
        let id = id.clone();
        tokio::spawn(async move {
            call(
                &app,
                Method::PATCH,
                &format!("/api/v1/webhooks/{id}"),
                &owner,
                Some(json!({"name": "Deleted"})),
            )
            .await
        })
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        paused.reached.notified(),
    )
    .await
    .expect("edit reached commit");
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/v1/webhooks/{id}"),
            &owner,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    paused.release.notify_one();
    assert_eq!(
        editing.await.expect("metadata task").0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            Method::GET,
            &format!("/api/v1/webhooks/{id}"),
            &owner,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}
