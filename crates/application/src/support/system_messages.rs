//! Group DM system messages (RECIPIENT_ADD/REMOVE, CHANNEL_NAME_CHANGE, CHANNEL_ICON_CHANGE),
//! posted through the messages service's internal endpoint.

use serde_json::{json, Value};

use domain::delivery_mode::DeliveryMode;
use domain::Snowflake;

use crate::context::Ports;

pub const RECIPIENT_ADD: u8 = 1;
pub const RECIPIENT_REMOVE: u8 = 2;
pub const CHANNEL_NAME_CHANGE: u8 = 4;
pub const CHANNEL_ICON_CHANGE: u8 = 5;

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
