//! HTTP tests for role hierarchy: members without ownership may not edit, reorder or assign
//! roles at or above their own highest role.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send};
use support::{jwt_keys, ports, start_database};

async fn register(app: &axum::Router, email: &str, username: &str) -> (String, String) {
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
    (
        body["access_token"].as_str().expect("token").to_owned(),
        body["user"]["id"].as_str().expect("id").to_owned(),
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

async fn make_role(
    app: &axum::Router,
    token: &str,
    guild: &str,
    name: &str,
    perms: &str,
) -> String {
    let (status, role) = call(
        app,
        Method::POST,
        &format!("/api/v1/guilds/{guild}/roles"),
        token,
        Some(json!({"name": name, "permissions": perms})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    role["id"].as_str().expect("role id").to_owned()
}

#[tokio::test]
async fn role_hierarchy_is_enforced_server_side() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let (owner, _) = register(&app, "owner@example.com", "owner").await;
    let (mod_token, mod_id) = register(&app, "mod@example.com", "moduser").await;
    let (target_token, target_id) = register(&app, "target@example.com", "target").await;

    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name": "HQ"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild_id = guild["id"].as_str().expect("guild id").to_owned();
    let (_, channels) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        None,
    )
    .await;
    let channel_id = channels[0]["id"].as_str().expect("channel").to_owned();

    // New roles land at the bottom, so create from the top of the hierarchy down.
    let high = make_role(&app, &owner, &guild_id, "high", "0").await;
    let mid = make_role(&app, &owner, &guild_id, "mid", "268435456").await; // MANAGE_ROLES
    let low = make_role(&app, &owner, &guild_id, "low", "0").await;

    join(&app, &owner, &channel_id, &mod_token).await;
    join(&app, &owner, &channel_id, &target_token).await;

    let roles_url = format!("/api/v1/guilds/{guild_id}/roles");
    let (status, _) = call(
        &app,
        Method::PUT,
        &format!("/api/v1/guilds/{guild_id}/members/{mod_id}/roles/{mid}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let assign = |role: &str| format!("/api/v1/guilds/{guild_id}/members/{target_id}/roles/{role}");
    let (status, _) = call(&app, Method::PUT, &assign(&high), &mod_token, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "cannot assign a role above");
    let (status, _) = call(&app, Method::PUT, &assign(&mid), &mod_token, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "cannot assign own rank");
    let (status, _) = call(&app, Method::PUT, &assign(&low), &mod_token, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "can assign a role below");
    let (status, _) = call(&app, Method::DELETE, &assign(&low), &mod_token, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "can remove a role below");
    let (status, _) = call(&app, Method::PUT, &assign(&high), &owner, None).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "the owner can assign any role"
    );
    let (status, _) = call(&app, Method::DELETE, &assign(&high), &mod_token, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "cannot remove a role above");

    for role in [&high, &mid] {
        let (status, _) = call(
            &app,
            Method::PATCH,
            &format!("{roles_url}/{role}"),
            &mod_token,
            Some(json!({"name": "x"})),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "cannot edit a role at or above"
        );
        let (status, _) = call(
            &app,
            Method::DELETE,
            &format!("{roles_url}/{role}"),
            &mod_token,
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "cannot delete a role at or above"
        );
    }
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("{roles_url}/{low}"),
        &mod_token,
        Some(json!({"permissions": "8"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "cannot grant permissions it lacks"
    );
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("{roles_url}/{low}"),
        &mod_token,
        Some(json!({"hoist": true, "mentionable": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "can edit a role below");

    let position_of = |list: &Value, id: &str| {
        list.as_array()
            .and_then(|l| l.iter().find(|r| r["id"] == id))
            .and_then(|r| r["position"].as_i64())
            .expect("position")
    };
    let (_, list) = call(&app, Method::GET, &roles_url, &mod_token, None).await;
    let high_pos = position_of(&list, &high);
    let mid_pos = position_of(&list, &mid);
    let low_pos = position_of(&list, &low);
    assert!(high_pos > mid_pos && mid_pos > low_pos);

    let (status, _) = call(
        &app,
        Method::PATCH,
        &roles_url,
        &mod_token,
        Some(json!([{"id": high, "position": 1}])),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "cannot move a role above");
    let (status, _) = call(
        &app,
        Method::PATCH,
        &roles_url,
        &mod_token,
        Some(json!([{"id": low, "position": mid_pos}])),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "cannot move a role to its own rank"
    );
    let (status, _) = call(
        &app,
        Method::PATCH,
        &roles_url,
        &mod_token,
        Some(json!([{"id": low, "position": high_pos + 1}])),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "cannot move a role above itself"
    );
    let (status, _) = call(
        &app,
        Method::PATCH,
        &roles_url,
        &mod_token,
        Some(json!([{"id": low, "position": low_pos}])),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "can move a role below");
    let (status, _) = call(
        &app,
        Method::PATCH,
        &roles_url,
        &owner,
        Some(json!([{"id": high, "position": 1}, {"id": low, "position": 9}])),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the owner can reorder freely");
}
