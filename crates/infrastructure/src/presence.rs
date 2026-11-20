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
    gateway: Option<(reqwest::Url, String, reqwest::Client)>,
}

impl ValkeyPresence {
    pub async fn connect(url: &str) -> AppResult<Self> {
        let client = redis::Client::open(url).map_err(AppError::internal)?;
        let connection = ConnectionManager::new(client)
            .await
            .map_err(|e| AppError::Unavailable(format!("valkey: {e}")))?;
        Ok(Self {
            connection,
            gateway: None,
        })
    }

    pub fn with_gateway(mut self, base: reqwest::Url, token: String) -> AppResult<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .map_err(AppError::internal)?;
        self.gateway = Some((base, token, client));
        Ok(self)
    }
}

fn unavailable(e: redis::RedisError) -> AppError {
    AppError::Unavailable(format!("valkey presence: {e}"))
}

fn parse_soundboard_grant(
    value: &serde_json::Value,
    now: std::time::SystemTime,
) -> AppResult<bool> {
    let invalid =
        || AppError::Unavailable("invalid or expired gateway publication response".into());
    let now_ms: u64 = now
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| invalid())?
        .as_millis()
        .try_into()
        .map_err(|_| invalid())?;
    let session = value
        .get("session_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(invalid)?;
    let _ = session;
    value
        .get("revision")
        .and_then(|v| v.as_u64())
        .ok_or_else(invalid)?;
    let expires_at = value
        .get("expires_at")
        .and_then(|v| v.as_u64())
        .ok_or_else(invalid)?;
    if expires_at <= now_ms {
        return Err(invalid());
    }
    let audio = value
        .get("can_publish_audio")
        .and_then(|v| v.as_bool())
        .ok_or_else(invalid)?;
    value
        .get("can_publish_video")
        .and_then(|v| v.as_bool())
        .ok_or_else(invalid)?;
    let soundboard = value
        .get("can_publish_soundboard")
        .and_then(|v| v.as_bool())
        .ok_or_else(invalid)?;
    Ok(audio && soundboard)
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
    async fn in_voice_channel(
        &self,
        guild_id: Snowflake,
        channel_id: Snowflake,
        user_id: Snowflake,
    ) -> AppResult<bool> {
        let (base, token, client) = self.gateway.as_ref().ok_or_else(|| {
            AppError::Unavailable("gateway voice lookup is not configured".into())
        })?;
        let url = base
            .join(&format!(
                "/internal/v1/guilds/{guild_id}/channels/{channel_id}/voice-members/{user_id}"
            ))
            .map_err(AppError::internal)?;
        let response = client
            .get(url)
            .header("X-Internal-Token", token)
            .send()
            .await
            .map_err(|_| AppError::Unavailable("gateway voice lookup failed".into()))?;
        if !response.status().is_success() {
            return Err(AppError::Unavailable("gateway voice lookup failed".into()));
        }
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|_| AppError::Unavailable("invalid gateway voice lookup response".into()))?;
        value
            .get("connected")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| AppError::Unavailable("invalid gateway voice lookup response".into()))
    }

    async fn can_publish_soundboard(
        &self,
        guild_id: Snowflake,
        channel_id: Snowflake,
        user_id: Snowflake,
    ) -> AppResult<bool> {
        let (base, token, client) = self.gateway.as_ref().ok_or_else(|| {
            AppError::Unavailable("gateway publication lookup is not configured".into())
        })?;
        let url = base.join(&format!(
            "/internal/v1/voice/guilds/{guild_id}/channels/{channel_id}/members/{user_id}/publish-permissions"
        )).map_err(AppError::internal)?;
        let response = client
            .get(url)
            .header("X-Internal-Token", token)
            .send()
            .await
            .map_err(|_| AppError::Unavailable("gateway publication lookup failed".into()))?;
        if !response.status().is_success() {
            return Err(AppError::Unavailable(
                "gateway publication lookup failed".into(),
            ));
        }
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|_| AppError::Unavailable("invalid gateway publication response".into()))?;
        parse_soundboard_grant(&value, std::time::SystemTime::now())
    }

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

    async fn live_ids(&self, user_ids: &[Snowflake]) -> AppResult<Vec<Snowflake>> {
        let mut connection = self.connection.clone();
        let mut live = Vec::new();
        for chunk in user_ids.chunks(CHUNK) {
            let mut pipe = redis::pipe();
            for id in chunk {
                pipe.cmd("EXISTS").arg(format!("presence:sessions:{id}"));
            }
            let counts: Vec<i64> = pipe
                .query_async(&mut connection)
                .await
                .map_err(unavailable)?;
            live.extend(
                chunk
                    .iter()
                    .zip(counts)
                    .filter(|(_, count)| *count > 0)
                    .map(|(id, _)| *id),
            );
        }
        Ok(live)
    }
}

#[cfg(test)]
mod tests {
    use super::{is_online, parse_soundboard_grant};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn soundboard_grant_requires_valid_live_audio_and_soundboard_permissions() {
        let now = UNIX_EPOCH + Duration::from_millis(1000);
        let mut grant = serde_json::json!({"session_id":"s", "revision":1, "expires_at":2000,
            "can_publish_audio":true, "can_publish_video":false, "can_publish_soundboard":true});
        assert!(parse_soundboard_grant(&grant, now).unwrap());
        grant["can_publish_audio"] = false.into();
        assert!(!parse_soundboard_grant(&grant, now).unwrap());
        grant["expires_at"] = 1000.into();
        assert!(parse_soundboard_grant(&grant, now).is_err());
        assert!(parse_soundboard_grant(&serde_json::json!({}), now).is_err());
    }

    #[test]
    fn offline_and_garbage_are_not_online() {
        assert!(is_online(r#"{"status":"online","activities":[]}"#));
        assert!(is_online(r#"{"status":"dnd"}"#));
        assert!(!is_online(r#"{"status":"offline"}"#));
        assert!(!is_online("not json"));
    }
}
