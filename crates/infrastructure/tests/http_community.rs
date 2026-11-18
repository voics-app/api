//! Community configuration and discoverable listing reports (api#45).
mod support;

use application::write_set::{Mutation, WriteSet};
use axum::http::{Method, StatusCode};
use domain::Snowflake;
use interface::state::AppState;
use serde_json::{json, Value};
use support::http::{call, request, send};
use support::{jwt_keys, ports, start_database};

fn router(database: &support::Database, budget: Option<u32>) -> (axum::Router, application::Ports) {
    let (private, public) = jwt_keys();
    let mut services = ports(&database.pool, &public, &private);
    services.settings.discovery_min_members = 1;
    services.settings.discovery_min_guild_age = chrono::Duration::zero();
    if let Some(budget) = budget {
        services.limiter = std::sync::Arc::new(support::BudgetLimiter::new(budget));
    }
    (
        interface::router(AppState {
            ports: services.clone(),
            internal_token: support::http::INTERNAL_TOKEN.into(),
            rate_limiter: std::sync::Arc::new(support::AllowAll),
            rate_policy: Default::default(),
        }),
        services,
    )
}

fn text_channel(guild: &Value) -> &str {
    guild["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == 0)
        .unwrap()["id"]
        .as_str()
        .unwrap()
}

async fn enable(app: &axum::Router, owner: &str, created: &Value) {
    let gid = created["id"].as_str().unwrap();
    let text = text_channel(created);
    let (status, result) = call(app, Method::PATCH, &format!("/api/v1/guilds/{gid}"), owner, Some(json!({"features": ["COMMUNITY"], "rules_channel_id": text, "public_updates_channel_id": text, "description": "A friendly public community"}))).await;
    assert_eq!(status, StatusCode::OK, "{result}");
}

async fn list(app: &axum::Router, owner: &str, gid: &str) {
    let (status, result) = call(
        app,
        Method::PUT,
        &format!("/api/v1/guilds/{gid}/discovery"),
        owner,
        Some(json!({"enabled": true, "category": "community", "tags": ["chat"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
}

async fn queue(app: &axum::Router) -> Value {
    let mut req = request(Method::GET, "/internal/v1/admin/reports", None, None);
    req.headers_mut().insert(
        "x-internal-token",
        support::http::INTERNAL_TOKEN.parse().unwrap(),
    );
    let (status, body) = send(app, req).await;
    assert_eq!(status, StatusCode::OK);
    body
}

async fn register(app: &axum::Router, name: &str) -> String {
    let (status, body) = send(app, request(Method::POST, "/api/v1/auth/register", None,
        Some(json!({"email": format!("{name}@example.com"), "username": name, "password": "long-enough-test-passphrase"})))).await;
    assert_eq!(status, StatusCode::CREATED);
    body["access_token"].as_str().unwrap().to_owned()
}

async fn guild(app: &axum::Router, token: &str) -> Value {
    let (status, guild) = call(
        app,
        Method::POST,
        "/api/v1/guilds",
        token,
        Some(json!({"name": "Community test"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    guild
}

#[tokio::test]
async fn community_requires_configured_channels_before_discovery() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let mut services = ports(&database.pool, &public, &private);
    services.settings.discovery_min_members = 1;
    services.settings.discovery_min_guild_age = chrono::Duration::zero();
    let app = interface::router(AppState {
        ports: services,
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: std::sync::Arc::new(support::AllowAll),
        rate_policy: Default::default(),
    });
    let owner = register(&app, "communityowner").await;
    let created = guild(&app, &owner).await;
    let gid = created["id"].as_str().unwrap();
    let text = created["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == 0)
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let path = format!("/api/v1/guilds/{gid}");
    let (status, enabled) = call(&app, Method::PATCH, &path, &owner,
        Some(json!({"features": ["COMMUNITY"], "rules_channel_id": text, "public_updates_channel_id": text, "description": "A friendly public community"}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(enabled["features"], json!(["COMMUNITY"]));
    assert_eq!(enabled["rules_channel_id"], text);
    assert_eq!(enabled["public_updates_channel_id"], text);
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/v1/channels/{text}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "replace configured channels before deleting them"
    );
    let (status, _) = call(
        &app,
        Method::PUT,
        &format!("{path}/discovery"),
        &owner,
        Some(json!({"enabled": true, "category": "community"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, disabled) = call(
        &app,
        Method::PATCH,
        &path,
        &owner,
        Some(json!({"features": []})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(disabled["features"], json!([]));
    assert!(disabled["rules_channel_id"].is_null());
    let (_, listing) = call(
        &app,
        Method::GET,
        &format!("{path}/discovery"),
        &owner,
        None,
    )
    .await;
    assert_eq!(listing["enabled"], false);
    assert!(listing["blockers"]
        .as_array()
        .unwrap()
        .contains(&json!("COMMUNITY_REQUIRED")));
}

#[tokio::test]
async fn community_validates_pair_permissions_and_channel_lifecycle() {
    let database = start_database().await;
    let (app, services) = router(&database, None);
    let owner = register(&app, "owner").await;
    let stranger = register(&app, "stranger").await;
    let created = guild(&app, &owner).await;
    let gid = created["id"].as_str().unwrap();
    let id: Snowflake = gid.parse().unwrap();
    let text = text_channel(&created);
    assert_eq!(created["features"], json!([]));
    assert!(created["rules_channel_id"].is_null());
    let foreign = guild(&app, &stranger).await;
    let path = format!("/api/v1/guilds/{gid}");
    for body in [
        json!({"features": ["COMMUNITY"]}),
        json!({"features": ["VERIFIED"]}),
        json!({"features": ["COMMUNITY", "COMMUNITY"]}),
        json!({"rules_channel_id": text}),
        json!({"features": ["COMMUNITY"], "rules_channel_id": text_channel(&foreign), "public_updates_channel_id": text}),
    ] {
        assert_eq!(
            call(&app, Method::PATCH, &path, &owner, Some(body.clone()))
                .await
                .0,
            StatusCode::BAD_REQUEST,
            "{body}"
        );
    }
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            &stranger,
            Some(json!({"features": []}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    // A regular member also lacks Manage Guild.
    let (_, invite) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{text}/invites"),
        &owner,
        Some(json!({})),
    )
    .await;
    let code = invite["code"].as_str().unwrap();
    assert_eq!(
        call(
            &app,
            Method::POST,
            &format!("/api/v1/invites/{code}"),
            &stranger,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            &stranger,
            Some(json!({"features": []}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let stale = services.guilds.find(id).await.unwrap().unwrap();
    enable(&app, &owner, &created).await;
    for body in [json!({"nsfw": true}), json!({"delivery_mode": "ephemeral"})] {
        assert_eq!(
            call(
                &app,
                Method::PATCH,
                &format!("/api/v1/channels/{text}"),
                &owner,
                Some(body)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            &owner,
            Some(json!({"rules_channel_id": null}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    // A normal guild write built before the Community change cannot reset it.
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::UpdateGuild(stale));
    services.unit_of_work.commit(writes).await.unwrap();
    assert!(services
        .guilds
        .find(id)
        .await
        .unwrap()
        .unwrap()
        .community
        .is_some());
    let (_, replacement) = call(
        &app,
        Method::POST,
        &format!("{path}/channels"),
        &owner,
        Some(json!({"name": "announcements", "type": 5})),
    )
    .await;
    let replacement = replacement["id"].as_str().unwrap();
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            &owner,
            Some(
                json!({"rules_channel_id": replacement, "public_updates_channel_id": replacement})
            )
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/v1/channels/{text}"),
            &owner,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, audit) = call(
        &app,
        Method::GET,
        &format!("{path}/audit-logs"),
        &owner,
        None,
    )
    .await;
    assert!(audit["audit_log_entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["changes"]
            .as_array()
            .is_some_and(|changes| changes.iter().any(|c| c["key"] == "rules_channel_id"))));
    assert_eq!(
        call(&app, Method::DELETE, &path, &owner, None).await.0,
        StatusCode::NO_CONTENT,
        "guild deletion cascades despite Community FKs"
    );
    assert!(services.guilds.find(id).await.unwrap().is_none());
}

#[tokio::test]
async fn invalid_channel_types_and_modes_cannot_enable_community() {
    let database = start_database().await;
    let (app, _) = router(&database, None);
    let owner = register(&app, "owner").await;
    let created = guild(&app, &owner).await;
    let gid = created["id"].as_str().unwrap();
    for channel in [
        json!({"name": "voice", "type": 2}),
        json!({"name": "adult", "type": 0, "nsfw": true}),
        json!({"name": "temporary", "type": 0, "delivery_mode": "ephemeral"}),
    ] {
        let (status, channel) = call(
            &app,
            Method::POST,
            &format!("/api/v1/guilds/{gid}/channels"),
            &owner,
            Some(channel),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _) = call(&app, Method::PATCH, &format!("/api/v1/guilds/{gid}"), &owner, Some(json!({"features": ["COMMUNITY"], "rules_channel_id": channel["id"], "public_updates_channel_id": text_channel(&created)}))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn listing_visibility_join_and_reports_share_eligibility() {
    let database = start_database().await;
    let (app, _) = router(&database, None);
    let owner = register(&app, "owner").await;
    let visitor = register(&app, "visitor").await;
    let created = guild(&app, &owner).await;
    let gid = created["id"].as_str().unwrap();
    let path = format!("/api/v1/guilds/{gid}");
    enable(&app, &owner, &created).await;
    list(&app, &owner, gid).await;
    assert_eq!(
        call(
            &app,
            Method::GET,
            "/api/v1/discovery/guilds",
            &visitor,
            None
        )
        .await
        .1["total"],
        1
    );
    // A short-but-nonempty description must hide the listing in SQL too.
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            &owner,
            Some(json!({"description": "short"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            Method::GET,
            "/api/v1/discovery/guilds",
            &visitor,
            None
        )
        .await
        .1["total"],
        0
    );
    for endpoint in ["join", "reports"] {
        assert_eq!(
            call(
                &app,
                Method::POST,
                &format!("/api/v1/discovery/guilds/{gid}/{endpoint}"),
                &visitor,
                Some(json!({"category": "spam"}))
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }
    enable(&app, &owner, &created).await;
    assert_eq!(
        call(
            &app,
            Method::GET,
            "/api/v1/discovery/guilds",
            &visitor,
            None
        )
        .await
        .1["total"],
        1
    );
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            &owner,
            Some(json!({"features": []}))
        )
        .await
        .0,
        StatusCode::OK
    );
    enable(&app, &owner, &created).await;
    assert_eq!(
        call(
            &app,
            Method::GET,
            "/api/v1/discovery/guilds",
            &visitor,
            None
        )
        .await
        .1["total"],
        0,
        "explicit disable removes opt-in"
    );
    list(&app, &owner, gid).await;
    assert_eq!(
        call(
            &app,
            Method::POST,
            &format!("/api/v1/discovery/guilds/{gid}/join"),
            &visitor,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn outsiders_report_public_snapshots_and_admins_review_them() {
    let database = start_database().await;
    let (app, _) = router(&database, None);
    let owner = register(&app, "owner").await;
    let visitor = register(&app, "visitor").await;
    let created = guild(&app, &owner).await;
    let gid = created["id"].as_str().unwrap();
    enable(&app, &owner, &created).await;
    list(&app, &owner, gid).await;
    let path = format!("/api/v1/discovery/guilds/{gid}/reports");
    let body = json!({"category": "spam", "note": "Public description is spam"});
    assert_eq!(
        send(&app, request(Method::POST, &path, None, Some(body.clone())))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, Method::POST, &path, &owner, Some(body.clone()))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    for invalid in [
        json!({"category": "invalid"}),
        json!({"category": "spam", "note": "x".repeat(501)}),
        json!({"category": "spam", "content": "forged"}),
    ] {
        assert_eq!(
            call(&app, Method::POST, &path, &visitor, Some(invalid))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    for _ in 0..2 {
        assert_eq!(
            call(&app, Method::POST, &path, &visitor, Some(body.clone()))
                .await
                .0,
            StatusCode::NO_CONTENT
        );
    }
    let page = queue(&app).await;
    assert_eq!(page["reports"].as_array().unwrap().len(), 1);
    let report = &page["reports"][0];
    assert_eq!(report["kind"], "guild");
    assert_eq!(report["guild_id"], gid);
    assert_eq!(report["reported"]["id"], created["owner_id"]);
    assert_eq!(report["content_source"], "server");
    assert_eq!(report["content"], "Name: Community test\nDescription: A friendly public community\nCategory: community\nTags: chat");
    for field in ["channel_id", "message_id", "delivery_mode"] {
        assert!(report[field].is_null());
    }
    let report_id = report["id"].as_str().unwrap();
    let mut req = request(
        Method::POST,
        &format!("/internal/v1/admin/reports/{report_id}/resolve"),
        None,
        Some(
            json!({"resolution": "resolved", "note": "Reviewed public listing", "resolved_by": "operator@example.com"}),
        ),
    );
    req.headers_mut().insert(
        "x-internal-token",
        support::http::INTERNAL_TOKEN.parse().unwrap(),
    );
    let (status, result) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["status"], "resolved");
    assert_eq!(
        call(
            &app,
            Method::GET,
            "/api/v1/discovery/guilds",
            &visitor,
            None
        )
        .await
        .1["total"],
        1,
        "review does not auto-delist"
    );
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/v1/guilds/{gid}"),
            &owner,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let content: String = sqlx::query_scalar("SELECT content FROM reports WHERE id = $1")
        .bind(report_id.parse::<i64>().unwrap())
        .fetch_one(&database.pool)
        .await
        .unwrap();
    assert!(
        content.starts_with("Name: Community test"),
        "evidence survives guild deletion"
    );
}

#[tokio::test]
async fn listing_and_profile_reports_share_the_reporter_budget() {
    let database = start_database().await;
    let (app, _) = router(&database, Some(2));
    let owner = register(&app, "owner").await;
    let visitor = register(&app, "visitor").await;
    let created = guild(&app, &owner).await;
    let gid = created["id"].as_str().unwrap();
    enable(&app, &owner, &created).await;
    list(&app, &owner, gid).await;
    assert_eq!(
        call(
            &app,
            Method::POST,
            "/api/v1/reports",
            &visitor,
            Some(json!({"target": "user", "user_id": created["owner_id"], "category": "spam"}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let path = format!("/api/v1/discovery/guilds/{gid}/reports");
    assert_eq!(
        call(
            &app,
            Method::POST,
            &path,
            &visitor,
            Some(json!({"category": "spam"}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            Method::POST,
            &path,
            &visitor,
            Some(json!({"category": "spam"}))
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn community_serializes_with_channel_edits_and_listing_changes() {
    let database = start_database().await;
    let (app, services) = router(&database, None);
    let owner = register(&app, "owner").await;
    let created = guild(&app, &owner).await;
    let gid: Snowflake = created["id"].as_str().unwrap().parse().unwrap();
    let cid: Snowflake = text_channel(&created).parse().unwrap();
    let mut channel = services.channels.find(cid).await.unwrap().unwrap();
    channel.nsfw = true;
    let mut config = WriteSet::new();
    config.mutate(Mutation::SetCommunity {
        guild_id: gid,
        channels: Some(domain::guild::CommunityChannels {
            rules_channel_id: cid,
            public_updates_channel_id: cid,
        }),
    });
    let mut change = WriteSet::new();
    change.mutate(Mutation::UpdateChannel(channel));
    let (configured, changed) = tokio::join!(
        services.unit_of_work.commit(config),
        services.unit_of_work.commit(change)
    );
    assert_ne!(
        configured.is_ok(),
        changed.is_ok(),
        "exactly one incompatible write may commit"
    );
    let current = services.guilds.find(gid).await.unwrap().unwrap();
    let channel = services.channels.find(cid).await.unwrap().unwrap();
    assert!(!(current.community.is_some() && channel.nsfw));
    let mut reset = WriteSet::new();
    reset.mutate(Mutation::SetCommunity {
        guild_id: gid,
        channels: None,
    });
    let mut channel = channel;
    channel.nsfw = false;
    reset.mutate(Mutation::UpdateChannel(channel));
    services.unit_of_work.commit(reset).await.unwrap();
    enable(&app, &owner, &created).await;
    let listing = domain::discovery::GuildDiscovery {
        category: "community".into(),
        tags: vec![],
        listed_at: services.clock.now(),
    };
    let mut disable = WriteSet::new();
    disable.mutate(Mutation::SetCommunity {
        guild_id: gid,
        channels: None,
    });
    let (disabled, listed) = tokio::join!(
        services.unit_of_work.commit(disable),
        services.guilds.set_discovery(gid, Some(&listing))
    );
    disabled.unwrap();
    // Listing either commits first and is removed, or observes Community disabled and refuses.
    if let Err(error) = listed {
        assert!(matches!(error, application::AppError::Validation(_)));
    }
    assert!(services.guilds.discovery(gid).await.unwrap().is_none());
}
