//! HTTP contract tests for server templates (api#44).

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

fn names(items: &Value) -> Vec<String> {
    items
        .as_array()
        .expect("array")
        .iter()
        .map(|i| i["name"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[tokio::test]
async fn builtin_templates_create_guilds_with_roles_channels_and_overwrites() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let alice = register(&app, "alice").await;

    let (status, list) = call(&app, Method::GET, "/api/v1/guilds/templates", &alice, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        names(&list),
        ["Gaming", "Friends", "Study Group", "Community", "Creators"]
    );
    assert_eq!(list[0]["code"], "builtin-gaming");
    assert!(list[0]["creator"].is_null());
    assert_eq!(list[0]["serialized_source_guild"]["roles"][0]["id"], 0);
    for template in list.as_array().unwrap() {
        let code = template["code"].as_str().unwrap();
        let (status, made) = call(
            &app,
            Method::POST,
            &format!("/api/v1/guilds/templates/{code}"),
            &alice,
            Some(json!({"name": "Template copy"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{code}: {made}");
        assert_eq!(made["members"].as_array().unwrap().len(), 1);
        for field in ["roles", "channels"] {
            let mut expected = names(&template["serialized_source_guild"][field]);
            let mut actual = names(&made[field]);
            expected.sort();
            actual.sort();
            assert_eq!(actual, expected, "{code}: {field}");
        }
    }

    let (status, one) = call(
        &app,
        Method::GET,
        "/api/v1/guilds/templates/builtin-community",
        &alice,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(one["name"], "Community");

    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds/templates/builtin-community",
        &alice,
        Some(json!({"name": "Hub"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(guild["name"], "Hub");
    assert_eq!(guild["owner_id"], guild["members"][0]["user"]["id"]);
    let role_names = names(&guild["roles"]);
    assert!(
        role_names.contains(&"Moderator".to_owned()),
        "{role_names:?}"
    );
    let channel_names = names(&guild["channels"]);
    for expected in ["rules", "general", "mod-chat", "Events", "Staff"] {
        assert!(
            channel_names.contains(&expected.to_owned()),
            "{expected} in {channel_names:?}"
        );
    }
    let mod_chat = guild["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "mod-chat")
        .unwrap();
    assert_eq!(
        mod_chat["permission_overwrites"].as_array().unwrap().len(),
        2
    );
    assert!(
        !mod_chat["parent_id"].is_null(),
        "mod-chat sits in the Staff category"
    );
    assert!(!guild["system_channel_id"].is_null());

    // The guild is a normal guild afterwards.
    let id = guild["id"].as_str().unwrap();
    let (status, _) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{id}"),
        &alice,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Validation and unknown codes.
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/guilds/templates/builtin-gaming",
        &alice,
        Some(json!({"name": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = call(
        &app,
        Method::POST,
        "/api/v1/guilds/templates/builtin-nope",
        &alice,
        Some(json!({"name": "Valid name"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], 10057);
    let (status, _) = send(
        &app,
        request(Method::GET, "/api/v1/guilds/templates", None, None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn shared_template_lifecycle() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let owner = register(&app, "owner").await;
    let member = register(&app, "member").await;
    let outsider = register(&app, "outsider").await;

    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name": "Source"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let gid = guild["id"].as_str().unwrap().to_owned();
    let text_channel = guild["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == 0)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (_, invite) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{text_channel}/invites"),
        &owner,
        Some(json!({})),
    )
    .await;
    let invite_code = invite["code"].as_str().unwrap();
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/v1/invites/{invite_code}"),
        &member,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Gate: MANAGE_GUILD only.
    let path = format!("/api/v1/guilds/{gid}/templates");
    for token in [&member, &outsider] {
        let (status, _) = call(&app, Method::GET, &path, token, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = call(&app, Method::POST, &path, token, Some(json!({"name": "T"}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    // Create.
    let (status, bad) = call(
        &app,
        Method::POST,
        &path,
        &owner,
        Some(json!({"name": " "})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(bad["code"], 50035);
    let (status, template) = call(
        &app,
        Method::POST,
        &path,
        &owner,
        Some(json!({"name": "My template", "description": "Start here"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let code = template["code"].as_str().unwrap().to_owned();
    assert!(!code.starts_with("builtin-"));
    assert_eq!(template["usage_count"], 0);
    assert_eq!(template["is_dirty"], false);
    assert_eq!(template["source_guild_id"], gid);
    assert_eq!(template["creator"]["username"], "owner");
    assert_eq!(template["serialized_source_guild"]["name"], "Source");
    assert_eq!(
        template["serialized_source_guild"]["channels"]
            .as_array()
            .unwrap()
            .len(),
        4
    );

    // One template per guild.
    let (status, dup) = call(
        &app,
        Method::POST,
        &path,
        &owner,
        Some(json!({"name": "Again"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(dup["code"], 30031);

    // A racing request can pass the use-case precheck; the adapter must preserve the same error.
    let store = ports(&database.pool, &public, &private).guilds;
    let mut duplicate = store.template(&code).await.unwrap().unwrap();
    duplicate.code = "race-duplicate-code".into();
    let error = store.insert_template(&duplicate).await.unwrap_err();
    assert!(
        matches!(error, application::AppError::BadRequest { code: 30031, .. }),
        "{error:?}"
    );

    let (status, list) = call(&app, Method::GET, &path, &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);

    // Anyone signed in can resolve the code.
    let code_path = format!("/api/v1/guilds/templates/{code}");
    let (status, seen) = call(&app, Method::GET, &code_path, &outsider, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(seen["name"], "My template");
    let (status, public_view) = send(&app, request(Method::GET, &code_path, None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(public_view["code"], code);
    assert!(public_view["creator"].get("email").is_none());

    // Modify.
    let template_path = format!("{path}/{code}");
    let (status, patched) = call(
        &app,
        Method::PATCH,
        &template_path,
        &owner,
        Some(json!({"name": "Renamed", "description": null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(patched["name"], "Renamed");
    assert!(patched["description"].is_null());
    let (status, _) = call(
        &app,
        Method::PATCH,
        &template_path,
        &member,
        Some(json!({"name": "Hijack"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Sync picks up a new channel.
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{gid}/channels"),
        &owner,
        Some(json!({"name": "extra", "type": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, dirty) = call(&app, Method::GET, &code_path, &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        dirty["is_dirty"], true,
        "source changes must mark the snapshot stale"
    );
    assert_eq!(
        dirty["serialized_source_guild"]["channels"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    let (_, dirty_list) = call(&app, Method::GET, &path, &owner, None).await;
    assert_eq!(dirty_list[0]["is_dirty"], true);
    let (status, synced) = call(&app, Method::PUT, &template_path, &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(synced["is_dirty"], false);
    assert_eq!(
        synced["serialized_source_guild"]["channels"]
            .as_array()
            .unwrap()
            .len(),
        5
    );

    // Create a guild from it: usage_count goes up.
    let (status, copy) = call(
        &app,
        Method::POST,
        &code_path,
        &outsider,
        Some(json!({"name": "Copy"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(copy["channels"].as_array().unwrap().len(), 5);
    assert_eq!(copy["owner_id"], copy["members"][0]["user"]["id"]);
    let (_, after) = call(&app, Method::GET, &code_path, &owner, None).await;
    assert_eq!(after["usage_count"], 1);

    // A template of another guild is not reachable through this guild's path.
    let (status, other) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name": "Other"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let other_id = other["id"].as_str().unwrap();
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/v1/guilds/{other_id}/templates/{code}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Delete.
    let (status, _) = call(&app, Method::DELETE, &template_path, &member, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&app, Method::DELETE, &template_path, &owner, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, gone) = call(&app, Method::GET, &code_path, &owner, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(gone["code"], 10057);
}

#[tokio::test]
async fn public_template_preview_does_not_authorize_creation() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    });
    let path = "/api/v1/guilds/templates/builtin-gaming";
    let (status, template) = send(&app, request(Method::GET, path, None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(template["name"], "Gaming");
    let (status, _) = send(
        &app,
        request(Method::POST, path, None, Some(json!({"name": "Copy"}))),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, missing) = send(
        &app,
        request(Method::GET, "/api/v1/guilds/templates/missing", None, None),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(missing["code"], 10057);
}
