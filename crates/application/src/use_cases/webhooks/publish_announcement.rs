use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::ports::StoredMessage;
use crate::support::{channel_access::ChannelAccess, throttle};
use crate::write_set::{Mutation, WriteSet};
use domain::channel::ChannelType;
use domain::delivery_mode::DeliveryMode;
use domain::permissions::Permissions;
use domain::Snowflake;
use std::time::Duration;

/// Reserves a publication and its per-target deliveries in one transaction.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    channel: Snowflake,
    message_id: Snowflake,
) -> AppResult<StoredMessage> {
    if !ports.settings.announcement_delivery_enabled {
        return Err(AppError::Unavailable(
            "announcement delivery is disabled".into(),
        ));
    }
    let access = ChannelAccess::visible(ports, channel, actor.user_id).await?;
    access.require(Permissions::READ_MESSAGE_HISTORY)?;
    if access.channel.kind != ChannelType::GuildAnnouncement
        || access.channel.delivery_mode != DeliveryMode::Standard
    {
        return Err(AppError::BadRequest {
            code: 50024,
            message: "Cannot publish in this channel".into(),
        });
    }
    let message = ports
        .messages
        .get(channel, message_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Message))?;
    let author = message
        .body
        .pointer("/author/id")
        .and_then(serde_json::Value::as_str)
        .and_then(|id| id.parse::<Snowflake>().ok())
        .ok_or_else(|| AppError::Unavailable("message author unavailable".into()))?;
    if author == actor.user_id {
        access.require(Permissions::SEND_MESSAGES)?;
    } else {
        access.require(Permissions::MANAGE_MESSAGES)?;
    }
    let flags = message
        .body
        .get("flags")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let kind = message
        .body
        .get("type")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if flags & (1 | 2 | 64) != 0 || !matches!(kind, 0 | 19) {
        return Err(AppError::BadRequest {
            code: 50024,
            message: "Cannot publish this message".into(),
        });
    }
    throttle::hit(
        ports,
        &format!("announcement-publish:{channel}"),
        10,
        Duration::from_secs(3600),
    )
    .await?;
    let followers = ports.webhooks.followers_of(channel).await?;
    let mut deliveries = Vec::with_capacity(followers.len());
    for follower in followers {
        deliveries.push((
            follower.target_channel_id,
            follower.webhook_id,
            ports.ids.next_id()?,
        ));
    }
    // Signed download URLs are short-lived bearer credentials; the durable
    // journal needs attachment identities, not those URLs.
    let mut payload = message.body.clone();
    if let Some(attachments) = payload
        .get_mut("attachments")
        .and_then(serde_json::Value::as_array_mut)
    {
        for attachment in attachments {
            if let Some(fields) = attachment.as_object_mut() {
                fields.remove("url");
                fields.remove("proxy_url");
            }
        }
    }
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::EnqueueAnnouncement {
        source_channel_id: channel,
        source_message_id: message_id,
        payload,
        deliveries,
    });
    ports.unit_of_work.commit(writes).await?;
    let marked = ports.messages.mark_crossposted(channel, message_id).await?;
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::MarkAnnouncementPublished {
        source_channel_id: channel,
        source_message_id: message_id,
    });
    ports.unit_of_work.commit(writes).await?;
    Ok(marked)
}
