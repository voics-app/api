//! System messages (group DM changes, guild member joins, thread creation),
//! posted through the messages service's internal endpoint.

use serde_json::{json, Value};

use domain::channel::Channel;
use domain::delivery_mode::DeliveryMode;
use domain::guild::Guild;
use domain::Snowflake;

use crate::context::Ports;

pub const RECIPIENT_ADD: u8 = 1;
pub const RECIPIENT_REMOVE: u8 = 2;
pub const CHANNEL_NAME_CHANGE: u8 = 4;
pub const CHANNEL_ICON_CHANGE: u8 = 5;
pub const GUILD_MEMBER_JOIN: u8 = 7;
pub const THREAD_CREATED: u8 = 18;

/// Posts a system message authored by `author_id`. Only channels in Standard delivery carry
/// server-stored messages, so Ephemeral and P2P groups get none (their clients show the
/// membership change from the gateway events). A failure never fails the request: the
/// membership change itself is already committed.
pub async fn post(
    ports: &Ports,
    mode: DeliveryMode,
    channel_id: Snowflake,
    author_id: Snowflake,
    kind: u8,
    target: Option<Snowflake>,
    content: Option<&str>,
) {
    if mode != DeliveryMode::Standard {
        return;
    }
    let mut body = json!({ "author_id": author_id.to_string(), "type": kind });
    if let Some(target) = target {
        body["target_user_id"] = Value::String(target.to_string());
    }
    if let Some(content) = content {
        body["content"] = Value::String(content.to_owned());
    }
    let _ = ports.messages.create(channel_id, body).await;
}

/// Announces a new member (type 7, authored by the member) in the guild's system channel when
/// the guild has one and joins are not suppressed. Best effort: the join is already committed.
pub async fn announce_join(ports: &Ports, guild: &Guild, member_id: Snowflake) {
    let Some(channel_id) = guild.system_channel_id.filter(|_| guild.announces_joins()) else {
        return;
    };
    let body = json!({ "author_id": member_id.to_string(), "type": GUILD_MEMBER_JOIN });
    if let Err(error) = ports.messages.create(channel_id, body).await {
        tracing::warn!(%channel_id, guild_id = %guild.id, %error, "failed to post the member join message");
    }
}

/// Announces a new thread (type 18, content is the thread name) in its parent channel. Best
/// effort: the thread is already committed.
pub async fn announce_thread(ports: &Ports, thread: &Channel, author_id: Snowflake) {
    let Some(parent_id) = thread.parent_id else {
        return;
    };
    let body = json!({
        "author_id": author_id.to_string(),
        "type": THREAD_CREATED,
        "thread_id": thread.id.to_string(),
        "content": thread.name.as_deref().unwrap_or_default(),
    });
    if let Err(error) = ports.messages.create(parent_id, body).await {
        tracing::warn!(%parent_id, thread_id = %thread.id, %error, "failed to post the thread created message");
    }
}
