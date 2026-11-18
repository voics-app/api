//! HTTP contract tests for `GET /guilds/{id}/application-command-index`.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports, start_database};

async fn register(app: &axum::Router, email: &str, username: &str) -> String {
    let (status, body) = send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({"email": email, "username": username, "password": "a-long-enough-passphrase"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "register {username}");
    body["access_token"].as_str().expect("token").to_owned()
}

async fn create_application(app: &axum::Router, token: &str, name: &str) -> (String, String) {
    let (status, created) = call(
        app,
        Method::POST,
        "/api/v1/applications",
        token,
        Some(json!({"name": name, "description": "Bot"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create {name}");
    assert_eq!(
        created["bot_id"], created["bot"]["id"],
        "applications expose the bot user id"
    );
    (
        created["id"].as_str().expect("application id").to_owned(),
        created["bot_id"].as_str().expect("bot id").to_owned(),
    )
}

async fn create_command(
    app: &axum::Router,
    token: &str,
    path: &str,
    name: &str,
    default_member_permissions: Option<&str>,
) {
    let mut body = json!({"name": name, "description": "Command", "type": 1});
    if let Some(permissions) = default_member_permissions {
        body["default_member_permissions"] = json!(permissions);
    }
    let (status, _) = call(app, Method::POST, path, token, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "create command {name}");
}

fn names(commands: &Value) -> Vec<String> {
    let mut names: Vec<String> = commands
        .as_array()
        .expect("commands array")
        .iter()
        .map(|command| command["name"].as_str().expect("name").to_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn command_index_lists_installed_bots_commands_filtered_by_permissions() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let owner = register(&app, "owner@example.com", "owner").await;
    let member = register(&app, "member@example.com", "member").await;
    let outsider = register(&app, "outsider@example.com", "outsider").await;

    let (helper_id, helper_bot) = create_application(&app, &owner, "Helper Bot").await;
    let (other_id, _) = create_application(&app, &owner, "Uninstalled Bot").await;

    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name": "Index"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild_id = guild["id"].as_str().expect("guild id").to_owned();

    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/oauth2/authorize",
        &owner,
        Some(json!({
            "client_id": helper_id,
            "guild_id": guild_id,
            "permissions": "8",
            "installation_revision": "absent:0",
            "scope": "bot applications.commands"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "install the helper bot");

    create_command(
        &app,
        &owner,
        &format!("/api/v1/applications/{helper_id}/commands"),
        "ping",
        None,
    )
    .await;
    create_command(
        &app,
        &owner,
        &format!("/api/v1/applications/{helper_id}/guilds/{guild_id}/commands"),
        "guildping",
        None,
    )
    .await;
    create_command(
        &app,
        &owner,
        &format!("/api/v1/applications/{helper_id}/commands"),
        "admin-only",
        Some("8"),
    )
    .await;
    create_command(
        &app,
        &owner,
        &format!("/api/v1/applications/{other_id}/commands"),
        "invisible",
        None,
    )
    .await;

    let (status, invite) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let channel_id = invite[0]["id"].as_str().expect("channel id").to_owned();
    let (status, invite) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{channel_id}/invites"),
        &owner,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let code = invite["code"].as_str().expect("code").to_owned();
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/v1/invites/{code}"),
        &member,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "member joins the guild");

    let index_path = format!("/api/v1/guilds/{guild_id}/application-command-index");

    let (status, index) = call(&app, Method::GET, &index_path, &member, None).await;
    assert_eq!(status, StatusCode::OK);
    let applications = index["applications"].as_array().expect("applications");
    assert_eq!(applications.len(), 1, "only installed bots are listed");
    assert_eq!(applications[0]["id"], helper_id.as_str());
    assert_eq!(applications[0]["name"], "Helper Bot");
    assert_eq!(applications[0]["bot_id"], helper_bot.as_str());
    assert!(applications[0].get("icon").is_some());
    assert_eq!(
        names(&index["application_commands"]),
        vec!["guildping".to_owned(), "ping".to_owned()],
        "global and guild commands, minus those the member may not invoke"
    );

    let (status, admin_index) = call(&app, Method::GET, &index_path, &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        names(&admin_index["application_commands"]),
        vec![
            "admin-only".to_owned(),
            "guildping".to_owned(),
            "ping".to_owned()
        ],
        "administrators see commands restricted to administrators"
    );

    let (status, _) = call(&app, Method::GET, &index_path, &outsider, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "non-members are refused");

    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/api/v1/guilds/{guild_id}/roles/{guild_id}"),
        &owner,
        Some(json!({"permissions": "0"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "strip @everyone permissions");

    let (status, body) = call(&app, Method::GET, &index_path, &member, None).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "members without USE_APPLICATION_COMMANDS are refused"
    );
    assert_eq!(body["code"], 50013);
}
