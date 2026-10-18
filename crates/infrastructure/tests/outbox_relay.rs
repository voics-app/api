//! Outbox relay against a real Redpanda broker (Kafka API) started with
//! testcontainers. Rows committed by the unit of work must reach the topic
//! with the P6 envelope as the record value, keyed by the partition key.

mod support;

use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use rskafka::client::partition::UnknownTopicHandling;
use rskafka::client::ClientBuilder;
use testcontainers::core::{ContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};

use application::events::DomainEvent;
use application::ports::UnitOfWork;
use application::write_set::WriteSet;
use contracts::envelope::{API_TOPICS, TOPIC_GUILD};
use domain::channel::{Channel, ChannelType};
use domain::Snowflake;
use infrastructure::events::kafka::KafkaPublisher;
use infrastructure::events::relay::OutboxRelay;
use infrastructure::postgres::PgUnitOfWork;

use support::start_database;

const REDPANDA_IMAGE: (&str, &str) = ("docker.redpanda.com/redpandadata/redpanda", "v24.2.7");

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|addr| addr.port())
        .expect("free port")
}

#[tokio::test]
async fn relay_publishes_committed_envelopes_to_redpanda() {
    let port = free_port();
    let advertised = format!("PLAINTEXT://127.0.0.1:{port}");
    let _broker = GenericImage::new(REDPANDA_IMAGE.0, REDPANDA_IMAGE.1)
        .with_exposed_port(ContainerPort::Tcp(9092))
        .with_wait_for(WaitFor::seconds(20))
        .with_mapped_port(port, ContainerPort::Tcp(9092))
        .with_cmd([
            "redpanda",
            "start",
            "--smp",
            "1",
            "--overprovisioned",
            "--memory",
            "1G",
            "--reserve-memory",
            "0M",
            "--node-id",
            "0",
            "--check=false",
            "--kafka-addr",
            "PLAINTEXT://0.0.0.0:9092",
            "--advertise-kafka-addr",
            advertised.as_str(),
        ])
        .start()
        .await
        .expect("start redpanda");

    let brokers = vec![format!("127.0.0.1:{port}")];
    let client = ClientBuilder::new(brokers.clone())
        .build()
        .await
        .expect("kafka client");
    let controller = client.controller_client().expect("controller");
    for topic in API_TOPICS {
        let mut created = false;
        for _ in 0..30 {
            if controller.create_topic(topic, 1, 1, 5_000).await.is_ok() {
                created = true;
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        assert!(created, "topic {topic} created");
    }

    let database = start_database().await;
    let uow = PgUnitOfWork::new(database.pool.clone(), "nl-home".into());
    let guild_id = Snowflake::new(4242);
    let now = chrono::Utc::now();
    let channel = Channel::new(
        Snowflake::new(4243),
        ChannelType::GuildText,
        Some(guild_id),
        now,
    );
    let mut writes = WriteSet::new();
    writes.emit(DomainEvent::ChannelCreate(channel));
    uow.commit(writes).await.expect("commit");

    let publisher = KafkaPublisher::connect(brokers.clone())
        .await
        .expect("publisher");
    let relay = OutboxRelay::new(database.pool.clone(), Arc::new(publisher));
    let relayed = relay.relay_batch().await.expect("relay");
    assert_eq!(relayed, 1);

    let (unsent,): (i64,) = sqlx::query_as("SELECT count(*) FROM outbox WHERE sent_at IS NULL")
        .fetch_one(&database.pool)
        .await
        .expect("count");
    assert_eq!(unsent, 0, "relayed rows are marked sent");

    let partition = client
        .partition_client(TOPIC_GUILD, 0, UnknownTopicHandling::Retry)
        .await
        .expect("partition client");
    let (records, _) = partition
        .fetch_records(0, 1..1_000_000, 2_000)
        .await
        .expect("fetch");
    assert_eq!(records.len(), 1);
    let record = &records[0].record;
    assert_eq!(record.key.as_deref(), Some(guild_id.to_string().as_bytes()));
    let envelope: serde_json::Value =
        serde_json::from_slice(record.value.as_deref().expect("value")).expect("json");
    assert_eq!(envelope["type"], "CHANNEL_CREATE");
    assert_eq!(envelope["guild_id"], guild_id.to_string());
    assert_eq!(envelope["v"], 1);
}
