//! HTTP contract tests for the moderation quick wins (api#101): duplicating a role, the
//! admin-only prune setting and the history of members who are no longer in the guild.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send};
use support::{jwt_keys, ports, start_database};

const MANAGE_ROLES: &str = "268435456";
const ROLE_CREATE: u64 = 30;
const MEMBER_KICK: u64 = 20;

fn app_for(database: &support::Database) -> axum::Router {
    let (private, public) = jwt_keys();
    interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    })
}

async fn register(app: &axum::Router, name: &str) -> (String, String) {
    let (status, body) = send(
        app,
        request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({"email": format!("{name}@example.com"), "username": name, "password": "a-long-enough-passphrase"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "register {name}");
    (
        body["access_token"].as_str().expect("token").to_owned(),
        body["user"]["id"].as_str().expect("id").to_owned(),
    )
}

/// Creates a guild and returns its id and the id of its first channel.
async fn make_guild(app: &axum::Router, owner: &str) -> (String, String) {
    let (status, guild) = call(
        app,
        Method::POST,
        "/api/v1/guilds",
        owner,
        Some(json!({"name": "HQ"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild_id = guild["id"].as_str().expect("guild id").to_owned();
    let (_, channels) = call(
        app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        owner,
        None,
    )
    .await;
    (
        guild_id,
        channels[0]["id"].as_str().expect("channel").to_owned(),
    )
}

async fn join(app: &axum::Router, owner: &str, channel: &str, token: &str) {
    let (status, invite) = call(
        app,
        Method::POST,
        &format!("/api/v1/channels/{channel}/invites"),
        owner,
        Some(json!({"max_uses": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let code = invite["code"].as_str().expect("code").to_owned();
    let (status, _) = call(
        app,
        Method::POST,
        &format!("/api/v1/invites/{code}"),
        token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

async fn make_role(app: &axum::Router, token: &str, guild: &str, body: Value) -> Value {
    let (status, role) = call(
        app,
        Method::POST,
        &format!("/api/v1/guilds/{guild}/roles"),
        token,
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    role
}

fn duplicate_url(guild: &str, role: &str) -> String {
    format!("/api/v1/guilds/{guild}/roles/{role}/duplicate")
}

#[tokio::test]
async fn duplicating_a_role_copies_it_just_below_the_source() {
    let database = start_database().await;
    let app = app_for(&database);
    let (owner, _) = register(&app, "owner").await;
    let (guild_id, _) = make_guild(&app, &owner).await;

    // New roles land at the bottom, so the top role is created first.
    make_role(&app, &owner, &guild_id, json!({"name": "top"})).await;
    let source = make_role(
        &app,
        &owner,
        &guild_id,
        json!({"name": "Moderators", "permissions": MANAGE_ROLES, "color": 255, "hoist": true, "mentionable": true}),
    )
    .await;
    make_role(&app, &owner, &guild_id, json!({"name": "bottom"})).await;

    let source_id = source["id"].as_str().unwrap();
    let (status, copy) = call(
        &app,
        Method::POST,
        &duplicate_url(&guild_id, source_id),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(copy["name"], "Moderators (copy)");
    assert_eq!(copy["permissions"], MANAGE_ROLES);
    assert_eq!(copy["color"], 255);
    assert_eq!(copy["hoist"], true);
    assert_eq!(copy["mentionable"], true);
    assert_ne!(copy["id"], source["id"]);

    let (_, roles) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/roles"),
        &owner,
        None,
    )
    .await;
    let mut ordered: Vec<(i64, String)> = roles
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["position"].as_i64().unwrap(),
                r["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    ordered.sort();
    let names: Vec<&str> = ordered.iter().map(|(_, name)| name.as_str()).collect();
    assert_eq!(
        names,
        [
            "@everyone",
            "bottom",
            "Moderators (copy)",
            "Moderators",
            "top"
        ],
        "the copy sits directly below the source"
    );

    let (_, log) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/audit-logs?action_type={ROLE_CREATE}&limit=1"),
        &owner,
        None,
    )
    .await;
    let entry = &log["audit_log_entries"][0];
    assert_eq!(entry["target_id"], copy["id"], "the copy is audit logged");
}

#[tokio::test]
async fn duplicating_a_role_respects_hierarchy_and_special_roles() {
    let database = start_database().await;
    let app = app_for(&database);
    let (owner, _) = register(&app, "owner").await;
    let (mod_token, mod_id) = register(&app, "moduser").await;
    let (guild_id, channel_id) = make_guild(&app, &owner).await;

    let high = make_role(&app, &owner, &guild_id, json!({"name": "high"})).await;
    let mid = make_role(
        &app,
        &owner,
        &guild_id,
        json!({"name": "mid", "permissions": MANAGE_ROLES}),
    )
    .await;
    let low = make_role(&app, &owner, &guild_id, json!({"name": "low"})).await;
    join(&app, &owner, &channel_id, &mod_token).await;
    let (status, _) = call(
        &app,
        Method::PUT,
        &format!(
            "/api/v1/guilds/{guild_id}/members/{mod_id}/roles/{}",
            mid["id"].as_str().unwrap()
        ),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    for (name, role) in [("above", &high), ("own rank", &mid)] {
        let (status, _) = call(
            &app,
            Method::POST,
            &duplicate_url(&guild_id, role["id"].as_str().unwrap()),
            &mod_token,
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "cannot duplicate a role {name}"
        );
    }
    let (status, copy) = call(
        &app,
        Method::POST,
        &duplicate_url(&guild_id, low["id"].as_str().unwrap()),
        &mod_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "can duplicate a role below");
    assert_eq!(copy["name"], "low (copy)");

    let (status, _) = call(
        &app,
        Method::POST,
        &duplicate_url(&guild_id, &guild_id),
        &owner,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "@everyone is not duplicated"
    );
    let (status, _) = call(
        &app,
        Method::POST,
        &duplicate_url(&guild_id, "1"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown role");
}

#[tokio::test]
async fn prune_admin_only_is_a_guild_setting_with_an_audit_trail() {
    let database = start_database().await;
    let app = app_for(&database);
    let (owner, _) = register(&app, "owner").await;
    let (guild_id, _) = make_guild(&app, &owner).await;
    let guild_url = format!("/api/v1/guilds/{guild_id}");

    let (_, guild) = call(&app, Method::GET, &guild_url, &owner, None).await;
    assert_eq!(guild["prune_admin_only"], false, "off by default");

    let (status, patched) = call(
        &app,
        Method::PATCH,
        &guild_url,
        &owner,
        Some(json!({"prune_admin_only": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(patched["prune_admin_only"], true);
    let (_, guild) = call(&app, Method::GET, &guild_url, &owner, None).await;
    assert_eq!(guild["prune_admin_only"], true, "persisted");

    let (_, log) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/audit-logs?action_type=1"),
        &owner,
        None,
    )
    .await;
    let changes = log["audit_log_entries"][0]["changes"].as_array().unwrap();
    assert!(changes
        .iter()
        .any(|c| c["key"] == "prune_admin_only" && c["new_value"] == true));
}

#[tokio::test]
async fn banned_and_kicked_users_stay_visible_in_bans_and_the_audit_log() {
    let database = start_database().await;
    let app = app_for(&database);
    let (owner, _) = register(&app, "owner").await;
    let (kicked_token, kicked_id) = register(&app, "kicked").await;
    let (banned_token, banned_id) = register(&app, "banned").await;
    let (guild_id, channel_id) = make_guild(&app, &owner).await;
    join(&app, &owner, &channel_id, &kicked_token).await;
    join(&app, &owner, &channel_id, &banned_token).await;

    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/v1/guilds/{guild_id}/members/{kicked_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        &app,
        Method::PUT,
        &format!("/api/v1/guilds/{guild_id}/bans/{banned_id}"),
        &owner,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, log) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/audit-logs?target_id={kicked_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entries = log["audit_log_entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "only entries about the kicked user");
    assert_eq!(entries[0]["action_type"], MEMBER_KICK);
    assert!(
        log["users"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["id"] == kicked_id.as_str() && u["username"] == "kicked"),
        "the kicked user's summary comes with the log"
    );

    let (_, bans) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/bans"),
        &owner,
        None,
    )
    .await;
    assert_eq!(bans[0]["user"]["username"], "banned");
    let (status, log) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/audit-logs?target_id={banned_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(log["audit_log_entries"].as_array().unwrap().len(), 1);
}
