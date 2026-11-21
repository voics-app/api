//! The api counts online members from the keys the gateway writes to Valkey.

use std::time::Duration;

use redis::AsyncCommands;
use testcontainers::core::{ContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};

use application::ports::PresenceDirectory;
use domain::Snowflake;
use infrastructure::presence::ValkeyPresence;

#[tokio::test]
async fn counts_only_users_with_a_live_non_offline_session() {
    let container = GenericImage::new("valkey/valkey", "8-alpine")
        .with_exposed_port(ContainerPort::Tcp(6379))
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .with_startup_timeout(Duration::from_secs(300))
        .start()
        .await
        .expect("start valkey");
    let port = container.get_host_port_ipv4(6379).await.expect("port");
    let url = format!("redis://127.0.0.1:{port}");
    let mut raw = redis::Client::open(url.as_str())
        .expect("client")
        .get_multiplexed_async_connection()
        .await
        .expect("connect");

    // 1 online with a session, 2 dnd with a session, 3 offline, 4 stale aggregate (no session), 5 unknown.
    for (id, status, session) in [
        (1, "online", true),
        (2, "dnd", true),
        (3, "offline", true),
        (4, "online", false),
    ] {
        let _: () = raw
            .set(
                format!("presence:agg:{id}"),
                format!(r#"{{"status":"{status}"}}"#),
            )
            .await
            .expect("set agg");
        if session {
            let _: () = raw
                .hset(format!("presence:sessions:{id}"), "s", "{}")
                .await
                .expect("hset");
        }
    }

    let presence = ValkeyPresence::connect(&url).await.expect("connect");
    let ids: Vec<Snowflake> = (1..=5).map(Snowflake::from_i64).collect();
    assert_eq!(presence.online_count(&ids).await.expect("count"), 2);
    assert_eq!(presence.online_count(&[]).await.expect("count"), 0);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = axum::Router::new().route(
        "/internal/v1/guilds/{guild}/channels/{channel}/voice-members/{user}",
        axum::routing::get(
            |axum::extract::Path((guild, channel, user)): axum::extract::Path<(
                String,
                String,
                String,
            )>,
             headers: axum::http::HeaderMap| async move {
                assert_eq!(headers.get("x-internal-token").unwrap(), "test-token");
                axum::Json(
                    serde_json::json!({"connected": guild == "1" && channel == "2" && user == "3"}),
                )
            },
        ),
    );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let presence = presence
        .with_gateway(
            format!("http://{addr}").parse().unwrap(),
            "test-token".into(),
        )
        .unwrap();
    assert!(presence
        .in_voice_channel(
            Snowflake::from_i64(1),
            Snowflake::from_i64(2),
            Snowflake::from_i64(3)
        )
        .await
        .unwrap());
    assert!(!presence
        .in_voice_channel(
            Snowflake::from_i64(1),
            Snowflake::from_i64(4),
            Snowflake::from_i64(3)
        )
        .await
        .unwrap());
    server.abort();
    let _ = server.await;
    assert!(presence
        .in_voice_channel(
            Snowflake::from_i64(1),
            Snowflake::from_i64(2),
            Snowflake::from_i64(3)
        )
        .await
        .is_err());
}
