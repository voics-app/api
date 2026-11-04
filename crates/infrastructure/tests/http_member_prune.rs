//! HTTP contract tests for member pruning (api#112): the dry-run count, the removal with one audit
//! entry, role filtering and the `prune_admin_only` guild setting.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send};
use support::{jwt_keys, ports, start_database};

const KICK_MEMBERS: &str = "2";
const ADMINISTRATOR: &str = "8";
const MEMBER_PRUNE: u64 = 21;

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

async fn grant(app: &axum::Router, owner: &str, guild: &str, user: &str, role: &str) {
    let (status, _) = call(
        app,
        Method::PUT,
        &format!("/api/v1/guilds/{guild}/members/{user}/roles/{role}"),
        owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// Pretends the user last used the app (and joined) `days` ago.
async fn age(database: &support::Database, user: &str, days: i32) {
    let id: i64 = user.parse().expect("snowflake");
    sqlx::query(
        "UPDATE sessions SET last_used_at = now() - make_interval(days => $2) WHERE user_id = $1",
    )
    .bind(id)
    .bind(days)
    .execute(&database.pool)
    .await
    .expect("age sessions");
    sqlx::query(
        "UPDATE members SET joined_at = now() - make_interval(days => $2) WHERE user_id = $1",
    )
    .bind(id)
    .bind(days)
    .execute(&database.pool)
    .await
    .expect("age membership");
}

fn prune_url(guild: &str) -> String {
    format!("/api/v1/guilds/{guild}/prune")
}

async fn is_member(app: &axum::Router, owner: &str, guild: &str, user: &str) -> bool {
    let (status, _) = call(
        app,
        Method::GET,
        &format!("/api/v1/guilds/{guild}/members/{user}"),
        owner,
        None,
    )
    .await;
    status == StatusCode::OK
}

#[tokio::test]
async fn prune_counts_then_removes_inactive_members_without_roles() {
    let database = start_database().await;
    let app = app_for(&database);
    let (owner, owner_id) = register(&app, "owner").await;
    let (idle_token, idle_id) = register(&app, "idle").await;
    let (roled_token, roled_id) = register(&app, "roled").await;
    let (active_token, active_id) = register(&app, "active").await;
    let (guild_id, channel_id) = make_guild(&app, &owner).await;
    for token in [&idle_token, &roled_token, &active_token] {
        join(&app, &owner, &channel_id, token).await;
    }
    let role = make_role(&app, &owner, &guild_id, json!({"name": "regular"})).await;
    let role_id = role["id"].as_str().unwrap();
    grant(&app, &owner, &guild_id, &roled_id, role_id).await;
    age(&database, &owner_id, 20).await;
    age(&database, &idle_id, 20).await;
    age(&database, &roled_id, 20).await;

    let (status, preview) = call(
        &app,
        Method::GET,
        &format!("{}?days=7", prune_url(&guild_id)),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        preview["pruned"], 1,
        "the owner, roled and active members are kept"
    );
    let (_, preview) = call(
        &app,
        Method::GET,
        &format!("{}?days=7&include_roles={role_id}", prune_url(&guild_id)),
        &owner,
        None,
    )
    .await;
    assert_eq!(
        preview["pruned"], 2,
        "an included role no longer protects its members"
    );
    let (_, preview) = call(
        &app,
        Method::GET,
        &format!("{}?days=30", prune_url(&guild_id)),
        &owner,
        None,
    )
    .await;
    assert_eq!(preview["pruned"], 0, "nobody was away for 30 days");
    assert!(
        is_member(&app, &owner, &guild_id, &idle_id).await,
        "a preview removes nobody"
    );

    let (status, done) = call(
        &app,
        Method::POST,
        &prune_url(&guild_id),
        &owner,
        Some(json!({"days": 7})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(done["pruned"], 1);
    assert!(!is_member(&app, &owner, &guild_id, &idle_id).await);
    assert!(is_member(&app, &owner, &guild_id, &roled_id).await);
    assert!(is_member(&app, &owner, &guild_id, &active_id).await);

    let (_, log) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/audit-logs?action_type={MEMBER_PRUNE}"),
        &owner,
        None,
    )
    .await;
    let entries = log["audit_log_entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "one entry for the whole prune");
    let changes = entries[0]["changes"].as_array().unwrap();
    assert!(changes
        .iter()
        .any(|c| c["key"] == "members_removed" && c["new_value"] == 1));
    assert!(changes
        .iter()
        .any(|c| c["key"] == "delete_member_days" && c["new_value"] == 7));
}

#[tokio::test]
async fn prune_needs_kick_members_or_administrator_when_admin_only() {
    let database = start_database().await;
    let app = app_for(&database);
    let (owner, _) = register(&app, "owner").await;
    let (kicker_token, kicker_id) = register(&app, "kicker").await;
    let (admin_token, admin_id) = register(&app, "admin").await;
    let (plain_token, _) = register(&app, "plain").await;
    let (guild_id, channel_id) = make_guild(&app, &owner).await;
    for token in [&kicker_token, &admin_token, &plain_token] {
        join(&app, &owner, &channel_id, token).await;
    }
    let kick = make_role(
        &app,
        &owner,
        &guild_id,
        json!({"name": "kick", "permissions": KICK_MEMBERS}),
    )
    .await;
    let admin = make_role(
        &app,
        &owner,
        &guild_id,
        json!({"name": "admin", "permissions": ADMINISTRATOR}),
    )
    .await;
    grant(
        &app,
        &owner,
        &guild_id,
        &kicker_id,
        kick["id"].as_str().unwrap(),
    )
    .await;
    grant(
        &app,
        &owner,
        &guild_id,
        &admin_id,
        admin["id"].as_str().unwrap(),
    )
    .await;
    let url = format!("{}?days=7", prune_url(&guild_id));

    let (status, _) = call(&app, Method::GET, &url, &plain_token, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "no permission");
    let (status, _) = call(
        &app,
        Method::POST,
        &prune_url(&guild_id),
        &kicker_token,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "KICK_MEMBERS and the default of 7 days"
    );
    let (status, _) = call(
        &app,
        Method::GET,
        &format!("{}?days=0", prune_url(&guild_id)),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "days out of range");

    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/api/v1/guilds/{guild_id}"),
        &owner,
        Some(json!({"prune_admin_only": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for (method, token) in [(Method::GET, &kicker_token), (Method::POST, &kicker_token)] {
        let uri = if method == Method::GET {
            url.clone()
        } else {
            prune_url(&guild_id)
        };
        let (status, _) = call(&app, method, &uri, token, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "admin only now");
    }
    let (status, _) = call(
        &app,
        Method::POST,
        &prune_url(&guild_id),
        &admin_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "administrators still can");
    let (status, _) = call(&app, Method::GET, &url, &owner, None).await;
    assert_eq!(status, StatusCode::OK, "so can the owner");
}

#[tokio::test]
async fn prune_never_removes_bots_which_have_no_login_sessions() {
    let database = start_database().await;
    let app = app_for(&database);
    let (owner, _) = register(&app, "owner").await;
    let (bot_token, bot_id) = register(&app, "bot").await;
    let (guild_id, channel_id) = make_guild(&app, &owner).await;
    join(&app, &owner, &channel_id, &bot_token).await;
    age(&database, &bot_id, 60).await;
    sqlx::query("UPDATE users SET bot = TRUE WHERE id = $1")
        .bind(bot_id.parse::<i64>().expect("snowflake"))
        .execute(&database.pool)
        .await
        .expect("mark bot");

    let (status, body) = call(
        &app,
        Method::POST,
        &prune_url(&guild_id),
        &owner,
        Some(json!({"days": 7})),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["pruned"], 0);
    assert!(is_member(&app, &owner, &guild_id, &bot_id).await);
}

#[tokio::test]
async fn prune_skips_members_the_caller_does_not_outrank() {
    let database = start_database().await;
    let app = app_for(&database);
    let (owner, _) = register(&app, "owner").await;
    let (mod_token, mod_id) = register(&app, "moderator").await;
    let (peer_token, peer_id) = register(&app, "peer").await;
    let (guild_id, channel_id) = make_guild(&app, &owner).await;
    join(&app, &owner, &channel_id, &mod_token).await;
    join(&app, &owner, &channel_id, &peer_token).await;
    let kick = make_role(
        &app,
        &owner,
        &guild_id,
        json!({"name": "kick", "permissions": KICK_MEMBERS}),
    )
    .await;
    let role_id = kick["id"].as_str().unwrap();
    grant(&app, &owner, &guild_id, &mod_id, role_id).await;
    grant(&app, &owner, &guild_id, &peer_id, role_id).await;
    age(&database, &peer_id, 60).await;
    let body = json!({"days": 7, "include_roles": [role_id]});

    let (status, count) = call(
        &app,
        Method::GET,
        &format!("{}?days=7&include_roles={role_id}", prune_url(&guild_id)),
        &mod_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(count["pruned"], 0, "the dry run matches what prune removes");

    let (status, removed) = call(
        &app,
        Method::POST,
        &prune_url(&guild_id),
        &mod_token,
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(removed["pruned"], 0);
    assert!(is_member(&app, &owner, &guild_id, &peer_id).await);

    let (status, removed) = call(
        &app,
        Method::POST,
        &prune_url(&guild_id),
        &owner,
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(removed["pruned"], 1, "the owner outranks everyone");
}
