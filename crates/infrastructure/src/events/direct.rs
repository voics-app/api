//! Direct publishing for events that need no transactional guarantee (typing).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;

use application::events::DomainEvent;
use application::ports::EventPublisher;
use application::{AppError, AppResult};

use super::kafka::KafkaPublisher;
use super::route::route;

/// Publishes straight to Redpanda, skipping the outbox.
pub struct KafkaEvents {
    publisher: Arc<KafkaPublisher>,
    default_region: String,
}

impl KafkaEvents {
    pub fn new(publisher: Arc<KafkaPublisher>, default_region: String) -> Self {
        Self {
            publisher,
            default_region,
        }
    }
}

#[async_trait]
impl EventPublisher for KafkaEvents {
    async fn publish(&self, event: DomainEvent) -> AppResult<()> {
        for routed in route(&event, &self.default_region, Utc::now()) {
            let value = serde_json::to_vec(&routed.envelope).map_err(AppError::internal)?;
            self.publisher
                .publish(routed.topic, &routed.key, value)
                .await?;
        }
        Ok(())
    }
}
