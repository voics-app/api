//! Redpanda (Kafka API) producer built on `rskafka`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use chrono::Utc;
use rskafka::client::partition::{Compression, PartitionClient, UnknownTopicHandling};
use rskafka::client::{Client, ClientBuilder};
use rskafka::record::Record;
use tokio::sync::Mutex;

use application::{AppError, AppResult};

/// Publishes records to the partition chosen by a stable hash of the key.
pub struct KafkaPublisher {
    client: Client,
    partitions: Mutex<HashMap<String, Vec<i32>>>,
    clients: Mutex<HashMap<(String, i32), Arc<PartitionClient>>>,
}

impl KafkaPublisher {
    pub async fn connect(brokers: Vec<String>) -> AppResult<Self> {
        let client = ClientBuilder::new(brokers)
            .client_id("discordy-api")
            .build()
            .await
            .map_err(|e| AppError::Unavailable(format!("kafka: {e}")))?;
        Ok(Self {
            client,
            partitions: Mutex::new(HashMap::new()),
            clients: Mutex::new(HashMap::new()),
        })
    }

    /// Produces `(key, value)` records to `topic`, keyed by `key`.
    pub async fn publish(&self, topic: &str, key: &str, value: Vec<u8>) -> AppResult<()> {
        let partition = self.partition_for(topic, key).await?;
        let client = self.partition_client(topic, partition).await?;
        let record = Record {
            key: Some(key.as_bytes().to_vec()),
            value: Some(value),
            headers: BTreeMap::new(),
            timestamp: Utc::now(),
        };
        client
            .produce(vec![record], Compression::NoCompression)
            .await
            .map_err(|e| AppError::Unavailable(format!("kafka produce: {e}")))?;
        Ok(())
    }

    async fn partition_for(&self, topic: &str, key: &str) -> AppResult<i32> {
        let mut partitions = self.partitions.lock().await;
        if !partitions.contains_key(topic) {
            let found = self
                .client
                .list_topics()
                .await
                .map_err(|e| AppError::Unavailable(format!("kafka metadata: {e}")))?;
            let list: Vec<i32> = found
                .into_iter()
                .find(|t| t.name == topic)
                .map(|t| t.partitions.into_iter().collect())
                .unwrap_or_default();
            if list.is_empty() {
                return Err(AppError::Unavailable(format!(
                    "kafka topic {topic} does not exist"
                )));
            }
            partitions.insert(topic.to_owned(), list);
        }
        let list = &partitions[topic];
        Ok(list[(fnv1a(key.as_bytes()) % list.len() as u64) as usize])
    }

    async fn partition_client(
        &self,
        topic: &str,
        partition: i32,
    ) -> AppResult<Arc<PartitionClient>> {
        let mut clients = self.clients.lock().await;
        if let Some(existing) = clients.get(&(topic.to_owned(), partition)) {
            return Ok(Arc::clone(existing));
        }
        let created = self
            .client
            .partition_client(topic, partition, UnknownTopicHandling::Retry)
            .await
            .map_err(|e| AppError::Unavailable(format!("kafka partition: {e}")))?;
        let created = Arc::new(created);
        clients.insert((topic.to_owned(), partition), Arc::clone(&created));
        Ok(created)
    }
}

/// FNV-1a: stable across processes, so a key always maps to the same partition.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::fnv1a;

    #[test]
    fn partitioning_hash_is_stable() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"guild-1") % 3, fnv1a(b"guild-1") % 3);
        assert_ne!(fnv1a(b"a"), fnv1a(b"b"));
    }
}
