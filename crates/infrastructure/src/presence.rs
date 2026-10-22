//! Read side of the gateway's presence in Valkey. The gateway keeps the last
//! broadcast aggregate in `presence:agg:<user_id>` and live sessions in the
//! expiring hash `presence:sessions:<user_id>`; a user counts as online when
//! both say so, so a crashed gateway's stale aggregate does not linger.

use async_trait::async_trait;
use redis::aio::ConnectionManager;

use application::ports::PresenceDirectory;
use application::{AppError, AppResult};
use domain::Snowflake;

const CHUNK: usize = 500;

#[derive(Clone)]
pub struct ValkeyPresence {
    connection: ConnectionManager,
}

impl ValkeyPresence {
    pub async fn connect(url: &str) -> AppResult<Self> {
        let client = redis::Client::open(url).map_err(AppError::internal)?;
        let connection = ConnectionManager::new(client)
            .await
            .map_err(|e| AppError::Unavailable(format!("valkey: {e}")))?;
        Ok(Self { connection })
    }
}

fn unavailable(e: redis::RedisError) -> AppError {
    AppError::Unavailable(format!("valkey presence: {e}"))
}

/// True when an aggregate payload shows a status other than offline.
fn is_online(aggregate: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(aggregate)
        .ok()
        .and_then(|v| v.get("status").and_then(|s| s.as_str()).map(str::to_owned))
        .is_some_and(|status| status != "offline")
}

#[async_trait]
impl PresenceDirectory for ValkeyPresence {
    async fn online_count(&self, user_ids: &[Snowflake]) -> AppResult<u64> {
        let mut connection = self.connection.clone();
        let mut online = 0u64;
        for chunk in user_ids.chunks(CHUNK) {
            let mut pipe = redis::pipe();
            pipe.cmd("MGET");
            for id in chunk {
                pipe.arg(format!("presence:agg:{id}"));
            }
            for id in chunk {
                pipe.cmd("EXISTS").arg(format!("presence:sessions:{id}"));
            }
            let (aggregates, live): (Vec<Option<String>>, Vec<i64>) = {
                // MGET answers first, then one EXISTS per user.
                let mut replies: Vec<redis::Value> = pipe
                    .query_async(&mut connection)
                    .await
                    .map_err(unavailable)?;
                let aggregates =
                    redis::from_redis_value(&replies.remove(0)).map_err(unavailable)?;
                let live = replies
                    .iter()
                    .map(|v| redis::from_redis_value(v).map_err(unavailable))
                    .collect::<AppResult<Vec<i64>>>()?;
                (aggregates, live)
            };
            online += aggregates
                .iter()
                .zip(live)
                .filter(|(aggregate, live)| {
                    *live > 0 && aggregate.as_deref().is_some_and(is_online)
                })
                .count() as u64;
        }
        Ok(online)
    }
}

#[cfg(test)]
mod tests {
    use super::is_online;

    #[test]
    fn offline_and_garbage_are_not_online() {
        assert!(is_online(r#"{"status":"online","activities":[]}"#));
        assert!(is_online(r#"{"status":"dnd"}"#));
        assert!(!is_online(r#"{"status":"offline"}"#));
        assert!(!is_online("not json"));
    }
}
