//! Profile persistence, authorization, asset keys and transactional events.
mod support;

use application::events::DomainEvent;
use application::ports::{AssetStorage, UnitOfWork};
use application::write_set::{Mutation, WriteSet};
use application::{AppResult, Ports};
use async_trait::async_trait;
use axum::http::{Method, StatusCode};
use domain::guild::Member;
use domain::Snowflake;
use interface::state::AppState;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use support::http::{call, request, send};

#[derive(Default)]
struct Assets(Mutex<Vec<(String, String)>>);
#[async_trait]
impl AssetStorage for Assets {
    async fn put(&self, key: &str, _: Vec<u8>, mime: &str) -> AppResult<()> {
        self.0.lock().unwrap().push((key.into(), mime.into()));
        Ok(())
    }
}

struct Fixture {
    database: support::Database,
    ports: Ports,
    app: axum::Router,
    assets: Arc<Assets>,
    owner: String,
    guest: String,
    guest_id: Snowflake,
    guild_id: Snowflake,
}

struct PausedProfile {
    inner: Arc<dyn UnitOfWork>,
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}

#[async_trait]
impl UnitOfWork for PausedProfile {
    async fn commit(&self, writes: WriteSet) -> AppResult<()> {
        if writes.mutations.iter().any(|mutation| {
            matches!(mutation,
            Mutation::SetMemberProfile { patch, .. } if patch.bio.is_some())
        }) {
            self.entered.notify_one();
            self.resume.notified().await;
        }
        self.inner.commit(writes).await
    }
}

#[tokio::test]
async fn profile_save_preserves_concurrent_moderation_in_storage_response_and_event() {
    let f = fixture().await;
    let paused = Arc::new(PausedProfile {
        inner: f.ports.unit_of_work.clone(),
        entered: tokio::sync::Notify::new(),
        resume: tokio::sync::Notify::new(),
    });
    let mut ports = f.ports.clone();
    ports.unit_of_work = paused.clone();
    let app = router(ports);
    let token = f.guest.clone();
    let path = f.member_path();
    let saving = tokio::spawn(async move {
        call(
            &app,
            Method::PATCH,
            &path,
            &token,
            Some(json!({"bio":"New server bio"})),
        )
        .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        paused.entered.notified(),
    )
    .await
    .unwrap();
    let (_, role) = call(
        &f.app,
        Method::POST,
        &format!("/api/v1/guilds/{}/roles", f.guild_id),
        &f.owner,
        Some(json!({"name":"Reader"})),
    )
    .await;
    let role_id = role["id"].as_str().unwrap();
    assert_eq!(
        f.patch(
            &f.owner,
            json!({"mute":true,"nick":"Moderator nickname","roles":[role_id]})
        )
        .await
        .0,
        StatusCode::OK
    );
    paused.resume.notify_one();
    let (status, saved) = saving.await.unwrap();
    assert_eq!(status, StatusCode::OK);
    let (_, current) = call(&f.app, Method::GET, &f.member_path(), &f.guest, None).await;
    let event = f.event("GUILD_MEMBER_UPDATE").await;
    for view in [saved, current, event] {
        assert_eq!(view["bio"], "New server bio");
        assert_eq!(view["mute"], true);
        assert_eq!(view["nick"], "Moderator nickname");
        assert_eq!(view["roles"], json!([role_id]));
    }
}

fn router(ports: Ports) -> axum::Router {
    interface::router(AppState {
        ports,
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: Arc::new(support::AllowAll),
        rate_policy: interface::state::RatePolicy::default(),
    })
}

async fn register(app: &axum::Router, name: &str) -> (String, Snowflake) {
    let (status, body) = send(app, request(Method::POST, "/api/v1/auth/register", None,
        Some(json!({"username": name, "email": format!("{name}@example.com"), "password": "a-long-enough-passphrase"})))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    (
        body["access_token"].as_str().unwrap().into(),
        body["user"]["id"].as_str().unwrap().parse().unwrap(),
    )
}

async fn fixture() -> Fixture {
    let database = support::start_database().await;
    let (private, public) = support::jwt_keys();
    let mut ports = support::ports(&database.pool, &public, &private);
    let assets = Arc::new(Assets::default());
    ports.assets = assets.clone();
    let app = router(ports.clone());
    let (owner, _) = register(&app, "owner").await;
    let (guest, guest_id) = register(&app, "guest").await;
    let (status, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name":"Profile tests"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild_id: Snowflake = guild["id"].as_str().unwrap().parse().unwrap();
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::InsertMember(Member::new(
        guild_id,
        guest_id,
        ports.clock.now(),
    )));
    ports.unit_of_work.commit(writes).await.unwrap();
    // Profile editing must work without the unrelated nickname permission.
    sqlx::query("UPDATE roles SET permissions = 0 WHERE id = $1")
        .bind(guild_id.as_i64())
        .execute(&database.pool)
        .await
        .unwrap();
    Fixture {
        database,
        ports,
        app,
        assets,
        owner,
        guest,
        guest_id,
        guild_id,
    }
}

impl Fixture {
    fn member_path(&self) -> String {
        format!("/api/v1/guilds/{}/members/{}", self.guild_id, self.guest_id)
    }
    async fn patch(&self, token: &str, body: Value) -> (StatusCode, Value) {
        call(
            &self.app,
            Method::PATCH,
            &self.member_path(),
            token,
            Some(body),
        )
        .await
    }
    async fn event(&self, kind: &str) -> Value {
        sqlx::query_scalar("SELECT payload->'data' FROM outbox WHERE payload->>'type' = $1 ORDER BY id DESC LIMIT 1")
            .bind(kind).fetch_one(&self.database.pool).await.unwrap()
    }
}

#[tokio::test]
async fn server_profiles_are_self_only_and_use_server_assets_with_global_fallback() {
    let f = fixture().await;
    let global = json!({"avatar":"data:image/png;base64,aW1hZ2U=", "banner":"data:image/webp;base64,aW1hZ2U=", "bio":"Global bio"});
    assert_eq!(
        call(
            &f.app,
            Method::PATCH,
            "/api/v1/users/@me",
            &f.guest,
            Some(global)
        )
        .await
        .0,
        StatusCode::OK
    );
    for body in [
        json!({"avatar":null}),
        json!({"banner":null}),
        json!({"bio":"Admin edit"}),
    ] {
        assert_eq!(f.patch(&f.owner, body).await.0, StatusCode::FORBIDDEN);
    }
    assert_eq!(
        f.patch(&f.guest, json!({"nick":"Not allowed"})).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, saved) = f.patch(&f.guest, json!({"avatar":"data:image/png;base64,aW1hZ2U=", "banner":"data:image/gif;base64,aW1hZ2U=", "bio":"  Server bio  "})).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["bio"], "Server bio");
    assert_eq!(saved["user"]["bio"], "Global bio");
    assert!(saved["user"].get("email").is_none());
    assert!(saved["user"].get("mfa_enabled").is_none());
    let avatar = saved["avatar"].as_str().unwrap();
    let banner = saved["banner"].as_str().unwrap();
    assert!(banner.starts_with("a_"));
    let objects = f.assets.0.lock().unwrap().clone();
    assert!(objects.contains(&(
        format!(
            "guilds/{}/users/{}/avatars/{avatar}.png",
            f.guild_id, f.guest_id
        ),
        "image/png".into()
    )));
    assert!(objects.contains(&(
        format!(
            "guilds/{}/users/{}/banners/{banner}.gif",
            f.guild_id, f.guest_id
        ),
        "image/gif".into()
    )));
    assert!(objects.iter().any(
        |(key, mime)| key.starts_with(&format!("banners/{}/", f.guest_id)) && mime == "image/webp"
    ));
    let event = f.event("GUILD_MEMBER_UPDATE").await;
    for field in ["avatar", "banner", "bio"] {
        assert_eq!(event[field], saved[field]);
    }
    let (_, listed) = call(
        &f.app,
        Method::GET,
        &format!("/api/v1/guilds/{}/members?limit=100", f.guild_id),
        &f.owner,
        None,
    )
    .await;
    let listed = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["user"]["id"] == f.guest_id.to_string())
        .unwrap();
    assert_eq!(listed["banner"], saved["banner"]);
    let (status, kept) = f
        .patch(&f.owner, json!({"mute":true,"nick":"Moderator nickname"}))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(kept["bio"], saved["bio"]);
    let (_, reset) = f
        .patch(&f.guest, json!({"avatar":null,"banner":null,"bio":"  "}))
        .await;
    for field in ["avatar", "banner", "bio"] {
        assert!(reset[field].is_null());
    }
    assert_eq!(reset["user"]["bio"], "Global bio");
    assert_eq!(reset["mute"], true);
    assert_eq!(reset["nick"], "Moderator nickname");
}

#[tokio::test]
async fn profile_validation_rejects_malformed_values_without_writing() {
    let f = fixture().await;
    for body in [
        json!({"pronouns":"x".repeat(41)}),
        json!({"theme_colors":[1]}),
        json!({"theme_colors":[1,2,3]}),
        json!({"theme_colors":[0,16777216]}),
        json!({"theme_colors":[-1,1]}),
        json!({"theme_colors":[1.5,1]}),
        json!({"accent_color":16777216}),
        json!({"banner":"https://example.com/image.png"}),
        json!({"banner":"data:image/svg+xml;base64,aW1hZ2U="}),
        json!({"banner":"data:image/png;base64,"}),
    ] {
        let (status, error) = call(
            &f.app,
            Method::PATCH,
            "/api/v1/users/@me",
            &f.guest,
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {error}");
    }
    for body in [
        json!({"bio":"x".repeat(301)}),
        json!({"avatar":"not an image"}),
        json!({"banner":"data:text/html;base64,aW1hZ2U="}),
    ] {
        assert_eq!(f.patch(&f.guest, body).await.0, StatusCode::BAD_REQUEST);
    }
    assert!(f.assets.0.lock().unwrap().is_empty());
    let (_, user) = call(&f.app, Method::GET, "/api/v1/users/@me", &f.guest, None).await;
    assert!(user["pronouns"].is_null());
    assert!(user["banner"].is_null());
    let (status, user) = call(
        &f.app,
        Method::PATCH,
        "/api/v1/users/@me",
        &f.guest,
        Some(json!({"pronouns":"é".repeat(40), "theme_colors":[0,16777215]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(user["pronouns"], "é".repeat(40));
}

#[tokio::test]
async fn stale_account_and_moderation_writes_preserve_saved_profiles_and_events() {
    let f = fixture().await;
    let stale_user = f.ports.users.find(f.guest_id).await.unwrap().unwrap();
    let mut stale_member = f
        .ports
        .guilds
        .member(f.guild_id, f.guest_id)
        .await
        .unwrap()
        .unwrap();
    let (_, saved) = call(&f.app, Method::PATCH, "/api/v1/users/@me", &f.guest, Some(json!({"pronouns":"they/them", "theme_colors":[0,16777215],"banner":"data:image/png;base64,aW1hZ2U=","accent_color":0}))).await;
    let (_, server) = f.patch(&f.guest, json!({"avatar":"data:image/png;base64,aW1hZ2U=","banner":"data:image/png;base64,aW1hZ2U=","bio":"Server"})).await;
    stale_member.mute = true;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpdateUser(stale_user.clone()))
        .mutate(Mutation::UpsertMember(stale_member.clone()))
        .emit(DomainEvent::UserUpdate(stale_user.clone()))
        .emit(DomainEvent::MemberUpdate(domain::guild::MemberProfile {
            user: stale_user,
            member: stale_member,
        }));
    f.ports.unit_of_work.commit(writes).await.unwrap();
    let (_, current) = call(&f.app, Method::GET, "/api/v1/users/@me", &f.guest, None).await;
    let event = f.event("USER_UPDATE").await;
    for field in ["pronouns", "banner", "theme_colors", "accent_color"] {
        assert_eq!(current[field], saved[field]);
        assert_eq!(event[field], saved[field]);
    }
    let (_, current) = call(&f.app, Method::GET, &f.member_path(), &f.guest, None).await;
    let event = f.event("GUILD_MEMBER_UPDATE").await;
    for field in ["avatar", "banner", "bio"] {
        assert_eq!(current[field], server[field]);
        assert_eq!(event[field], server[field]);
    }
    assert_eq!(current["mute"], true);
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::AnonymizeUser {
        user_id: f.guest_id,
        username: "deleted-user".into(),
    });
    f.ports.unit_of_work.commit(writes).await.unwrap();
    let (_, deleted) = call(
        &f.app,
        Method::GET,
        &format!("/api/v1/users/{}", f.guest_id),
        &f.owner,
        None,
    )
    .await;
    for field in [
        "avatar",
        "banner",
        "bio",
        "pronouns",
        "theme_colors",
        "accent_color",
    ] {
        assert!(deleted[field].is_null());
    }
    let member = f
        .ports
        .guilds
        .member(f.guild_id, f.guest_id)
        .await
        .unwrap()
        .unwrap();
    assert!(member.avatar.is_none() && member.banner.is_none() && member.bio.is_none());
}

#[tokio::test]
async fn failed_member_event_rolls_back_profile_and_rejoin_resets_overrides() {
    let f = fixture().await;
    sqlx::query("ALTER TABLE outbox ADD CONSTRAINT reject_profile CHECK (payload->>'type' <> 'GUILD_MEMBER_UPDATE')")
        .execute(&f.database.pool).await.unwrap();
    assert_eq!(
        f.patch(&f.guest, json!({"bio":"Must roll back"})).await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert!(f
        .ports
        .guilds
        .member(f.guild_id, f.guest_id)
        .await
        .unwrap()
        .unwrap()
        .bio
        .is_none());
    sqlx::query("ALTER TABLE outbox DROP CONSTRAINT reject_profile")
        .execute(&f.database.pool)
        .await
        .unwrap();
    assert_eq!(
        f.patch(&f.guest, json!({"bio":"Server only"})).await.0,
        StatusCode::OK
    );
    let (_, other) = call(
        &f.app,
        Method::POST,
        "/api/v1/guilds",
        &f.guest,
        Some(json!({"name":"Other server"})),
    )
    .await;
    let other_id: Snowflake = other["id"].as_str().unwrap().parse().unwrap();
    assert!(f
        .ports
        .guilds
        .member(other_id, f.guest_id)
        .await
        .unwrap()
        .unwrap()
        .bio
        .is_none());
    assert_eq!(
        call(
            &f.app,
            Method::DELETE,
            &format!("/api/v1/users/@me/guilds/{}", f.guild_id),
            &f.guest,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::InsertMember(Member::new(
        f.guild_id,
        f.guest_id,
        f.ports.clock.now(),
    )));
    f.ports.unit_of_work.commit(writes).await.unwrap();
    assert!(f
        .ports
        .guilds
        .member(f.guild_id, f.guest_id)
        .await
        .unwrap()
        .unwrap()
        .bio
        .is_none());
}
