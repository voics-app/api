//! HTTP contract tests for server discovery (api#74).

mod support;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use interface::state::AppState;
use support::http::{call, request, send};
use support::{jwt_keys, ports, start_database};

async fn register(app: &axum::Router, name: &str) -> String {
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
    body["access_token"].as_str().expect("token").to_owned()
}

async fn make_guild(app: &axum::Router, token: &str, name: &str, description: &str) -> String {
    let (status, guild) = call(
        app,
        Method::POST,
        "/api/v1/guilds",
        token,
        Some(json!({"name": name})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = guild["id"].as_str().expect("id").to_owned();
    let text = guild["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == 0)
        .unwrap()["id"]
        .clone();
    let (status, _) = call(
        app,
        Method::PATCH,
        &format!("/api/v1/guilds/{id}"),
        token,
        Some(json!({"description": description, "features": ["COMMUNITY"], "rules_channel_id": text, "public_updates_channel_id": text})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    id
}

async fn put_discovery(
    app: &axum::Router,
    token: &str,
    guild: &str,
    body: Value,
) -> (StatusCode, Value) {
    call(
        app,
        Method::PUT,
        &format!("/api/v1/guilds/{guild}/discovery"),
        token,
        Some(body),
    )
    .await
}

#[tokio::test]
async fn discovery_lists_filters_and_joins() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let mut ports = ports(&database.pool, &public, &private);
    ports.settings.discovery_min_members = 1;
    ports.settings.discovery_min_guild_age = chrono::Duration::zero();
    let app = interface::router(AppState {
        ports,
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let owner = register(&app, "owner").await;
    let visitor = register(&app, "visitor").await;

    let rustaceans = make_guild(
        &app,
        &owner,
        "Rustaceans",
        "Everything about the Rust language",
    )
    .await;
    let gamers = make_guild(
        &app,
        &owner,
        "Night Gamers",
        "Late night co-op gaming sessions",
    )
    .await;
    let bare = make_guild(&app, &owner, "No Description", "").await;

    // Nothing is listed by default.
    let (status, page) = call(
        &app,
        Method::GET,
        "/api/v1/discovery/guilds",
        &visitor,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["total"], 0);

    // Validation and eligibility.
    let (status, _) = put_discovery(
        &app,
        &owner,
        &rustaceans,
        json!({"enabled": true, "category": "memes"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = put_discovery(
        &app,
        &owner,
        &bare,
        json!({"enabled": true, "category": "gaming"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "needs a description");
    let (status, _) = put_discovery(
        &app,
        &visitor,
        &rustaceans,
        json!({"enabled": true, "category": "gaming"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "non-members cannot manage discovery"
    );

    let (status, settings) = put_discovery(
        &app,
        &owner,
        &rustaceans,
        json!({"enabled": true, "category": "science_tech", "tags": ["Rust", "open-source", "rust"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settings["enabled"], true);
    assert_eq!(settings["tags"], json!(["rust", "open-source"]));
    let (status, _) = put_discovery(
        &app,
        &owner,
        &gamers,
        json!({"enabled": true, "category": "gaming", "tags": ["coop"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, settings) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{bare}/discovery"),
        &owner,
        None,
    )
    .await;
    assert_eq!(settings["enabled"], false);
    assert_eq!(settings["blockers"], json!(["DESCRIPTION_REQUIRED"]));

    // Browse, search and filter.
    let (_, page) = call(
        &app,
        Method::GET,
        "/api/v1/discovery/guilds",
        &visitor,
        None,
    )
    .await;
    assert_eq!(page["total"], 2);
    assert_eq!(page["guilds"][0]["approximate_member_count"], 1);
    let (_, by_text) = call(
        &app,
        Method::GET,
        "/api/v1/discovery/guilds?query=rust",
        &visitor,
        None,
    )
    .await;
    assert_eq!(by_text["guilds"].as_array().expect("list").len(), 1);
    assert_eq!(by_text["guilds"][0]["name"], "Rustaceans");
    let (_, by_category) = call(
        &app,
        Method::GET,
        "/api/v1/discovery/guilds?category=gaming",
        &visitor,
        None,
    )
    .await;
    assert_eq!(by_category["guilds"][0]["name"], "Night Gamers");
    let (_, by_tag) = call(
        &app,
        Method::GET,
        "/api/v1/discovery/guilds?tag=open-source",
        &visitor,
        None,
    )
    .await;
    assert_eq!(by_tag["guilds"][0]["name"], "Rustaceans");
    let (_, paged) = call(
        &app,
        Method::GET,
        "/api/v1/discovery/guilds?limit=1&offset=1",
        &visitor,
        None,
    )
    .await;
    assert_eq!(paged["total"], 2);
    assert_eq!(paged["guilds"].as_array().expect("list").len(), 1);
    let (_, categories) = call(
        &app,
        Method::GET,
        "/api/v1/discovery/categories",
        &visitor,
        None,
    )
    .await;
    assert!(categories["categories"]
        .as_array()
        .expect("list")
        .contains(&json!("gaming")));

    // An NSFW channel hides the guild from search and blocks joining.
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{gamers}/channels"),
        &owner,
        Some(json!({"name": "spicy", "type": 0, "nsfw": true})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, page) = call(
        &app,
        Method::GET,
        "/api/v1/discovery/guilds",
        &visitor,
        None,
    )
    .await;
    assert_eq!(page["total"], 1);
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/v1/discovery/guilds/{gamers}/join"),
        &visitor,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Join from discovery, then it counts as a member.
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/v1/discovery/guilds/{rustaceans}/join"),
        &visitor,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, mine) = call(
        &app,
        Method::GET,
        "/api/v1/users/@me/guilds",
        &visitor,
        None,
    )
    .await;
    assert!(mine
        .as_array()
        .expect("list")
        .iter()
        .any(|g| g["id"] == json!(rustaceans)));
    let (_, page) = call(
        &app,
        Method::GET,
        "/api/v1/discovery/guilds",
        &visitor,
        None,
    )
    .await;
    assert_eq!(page["guilds"][0]["approximate_member_count"], 2);

    // Unlisted guilds cannot be joined this way; delisting hides the guild.
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/v1/discovery/guilds/{bare}/join"),
        &visitor,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, settings) =
        put_discovery(&app, &owner, &rustaceans, json!({"enabled": false})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settings["enabled"], false);
    let (_, page) = call(
        &app,
        Method::GET,
        "/api/v1/discovery/guilds",
        &visitor,
        None,
    )
    .await;
    assert_eq!(page["total"], 0);
}

#[tokio::test]
async fn default_rules_keep_new_small_guilds_out() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let owner = register(&app, "owner").await;
    let guild = make_guild(&app, &owner, "Tiny", "A brand new small server").await;
    let (status, _) = put_discovery(
        &app,
        &owner,
        &guild,
        json!({"enabled": true, "category": "other"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, settings) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild}/discovery"),
        &owner,
        None,
    )
    .await;
    assert_eq!(
        settings["blockers"],
        json!(["MEMBER_COUNT_TOO_LOW", "GUILD_TOO_NEW"])
    );
}
