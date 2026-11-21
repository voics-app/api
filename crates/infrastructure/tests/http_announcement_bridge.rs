//! Cross-repository bridge: real API HTTP, PostgreSQL, and a loopback instance
//! of the real messages router. Run with MESSAGES_FIXTURE_BIN set.
mod support;

use application::ports::{MessageService, StoredMessage};
use application::{AppError, AppResult};
use async_trait::async_trait;
use axum::http::{Method, StatusCode};
use domain::Snowflake;
use infrastructure::{announcement_relay::AnnouncementRelay, messages_client::HttpMessages};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};
use support::http::{call, request, send, INTERNAL_TOKEN};
use support::{jwt_keys, ports_with_messages, start_database};

struct Fixture(Child);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct LostResponse {
    inner: HttpMessages,
    lose: AtomicBool,
    sends: AtomicUsize,
}
#[async_trait]
impl MessageService for LostResponse {
    async fn create(&self, c: Snowflake, m: Value) -> AppResult<StoredMessage> {
        self.inner.create(c, m).await
    }
    async fn edit(&self, c: Snowflake, m: Snowflake, p: Value) -> AppResult<StoredMessage> {
        self.inner.edit(c, m, p).await
    }
    async fn delete(&self, c: Snowflake, m: Snowflake) -> AppResult<()> {
        self.inner.delete(c, m).await
    }
    async fn get(&self, c: Snowflake, m: Snowflake) -> AppResult<Option<StoredMessage>> {
        self.inner.get(c, m).await
    }
    async fn mark_crossposted(&self, c: Snowflake, m: Snowflake) -> AppResult<StoredMessage> {
        self.inner.mark_crossposted(c, m).await
    }
    async fn deliver_crosspost(&self, c: Snowflake, body: Value) -> AppResult<StoredMessage> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        let result = self.inner.deliver_crosspost(c, body).await?;
        if self.lose.swap(false, Ordering::SeqCst) {
            Err(AppError::Unavailable("fixture lost the response".into()))
        } else {
            Ok(result)
        }
    }
}

#[tokio::test]
#[ignore = "requires the separately built messages loopback fixture"]
async fn real_services_reconcile_a_lost_delivery_response() {
    let executable = std::env::var("MESSAGES_FIXTURE_BIN")
        .expect("build the messages announcement_fixture example");
    let mut child = Command::new(executable)
        .stdout(Stdio::piped())
        .spawn()
        .expect("start messages fixture");
    let mut address = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    let _fixture = Fixture(child);
    let base = format!("http://{}/", address.trim());
    let messages = Arc::new(LostResponse {
        inner: HttpMessages::new(base.parse().unwrap(), INTERNAL_TOKEN.into()),
        lose: AtomicBool::new(true),
        sends: AtomicUsize::new(0),
    });
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let mut ports = ports_with_messages(&database.pool, &public, &private, messages.clone());
    ports.settings.announcement_delivery_enabled = true;
    let app = interface::router(interface::state::AppState {
        ports,
        internal_token: INTERNAL_TOKEN.into(),
        rate_limiter: Arc::new(support::AllowAll),
        rate_policy: Default::default(),
    });
    let (status, user) = send(&app, request(Method::POST, "/api/v1/auth/register", None, Some(json!({"email":"bridge@example.com", "username":"bridge", "password":"a-long-enough-passphrase"})))).await;
    assert_eq!(status, StatusCode::CREATED);
    let token = user["access_token"].as_str().unwrap();
    let (_, source_guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        token,
        Some(json!({"name":"Source"})),
    )
    .await;
    let (_, target_guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        token,
        Some(json!({"name":"Target"})),
    )
    .await;
    let source_guild_id = source_guild["id"].as_str().unwrap();
    let target_guild_id = target_guild["id"].as_str().unwrap();
    let (status, source) = call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{source_guild_id}/channels"),
        token,
        Some(json!({"name":"news", "type":5})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let source_id = source["id"].as_str().unwrap();
    let (_, channels) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{target_guild_id}/channels"),
        token,
        None,
    )
    .await;
    let target_id = channels
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == 0)
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let (status, followed) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{source_id}/followers"),
        token,
        Some(json!({"webhook_channel_id":target_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{followed}");
    let author_id = source_guild["owner_id"].as_str().unwrap();
    reqwest::Client::new().post(format!("{base}fixture/seed")).json(&json!({"channel_id":source_id, "message_id":"12345", "author_id":author_id, "content":"real bridge"})).send().await.unwrap().error_for_status().unwrap();
    let (status, source_message) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{source_id}/messages/12345/crosspost"),
        token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{source_message}");
    assert_eq!(source_message["flags"].as_u64().unwrap() & 1, 1);
    let relay = AnnouncementRelay::new(database.pool.clone(), messages.clone());
    relay.run_once().await.unwrap();
    let row: (i64, String) = sqlx::query_as("SELECT destination_message_id, state FROM announcement_deliveries WHERE source_channel_id=$1 AND source_message_id=12345")
        .bind(source_id.parse::<i64>().unwrap()).fetch_one(&database.pool).await.unwrap();
    assert_eq!(row.1, "unknown");
    let delivered = messages
        .get(target_id.parse().unwrap(), Snowflake::new(row.0 as u64))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(delivered.body["content"], "real bridge");
    assert_eq!(delivered.body["flags"].as_u64().unwrap() & 2, 2);
    assert_eq!(delivered.body["webhook_id"], followed["webhook_id"]);
    relay.run_once().await.unwrap();
    let state: String = sqlx::query_scalar("SELECT state FROM announcement_deliveries WHERE source_channel_id=$1 AND source_message_id=12345")
        .bind(source_id.parse::<i64>().unwrap()).fetch_one(&database.pool).await.unwrap();
    assert_eq!(state, "delivered");
    assert_eq!(messages.sends.load(Ordering::SeqCst), 1);
}
