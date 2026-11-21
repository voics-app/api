use domain::application::{Webhook, WebhookKind};
use domain::channel::ChannelType;
use domain::delivery_mode::DeliveryMode;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::support::channel_access::ChannelAccess;
use crate::write_set::{Mutation, WriteSet};

/// Creates a non-executable follower webhook and its relationship atomically.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    source: Snowflake,
    target: Snowflake,
) -> AppResult<Webhook> {
    if !ports.settings.announcement_delivery_enabled {
        return Err(AppError::Unavailable(
            "announcement delivery is disabled".into(),
        ));
    }
    let source_access = ChannelAccess::visible(ports, source, actor.user_id).await?;
    let target_access = ChannelAccess::visible(ports, target, actor.user_id).await?;
    target_access.require(Permissions::MANAGE_WEBHOOKS)?;
    let source_guild = source_access.guild()?.guild.id;
    let target_guild = target_access.guild()?.guild.id;
    if source_access.channel.kind != ChannelType::GuildAnnouncement
        || target_access.channel.kind != ChannelType::GuildText
        || source_access.channel.delivery_mode != DeliveryMode::Standard
        || target_access.channel.delivery_mode != DeliveryMode::Standard
        || source_guild == target_guild
    {
        return Err(AppError::BadRequest {
            code: 50024,
            message: "Cannot follow into this channel".into(),
        });
    }
    let webhook = Webhook {
        id: ports.ids.next_id()?,
        kind: WebhookKind::ChannelFollower,
        guild_id: target_guild,
        channel_id: target,
        creator_id: Some(actor.user_id),
        name: source_access
            .channel
            .name
            .unwrap_or_else(|| "Announcement".into()),
        avatar: None,
        application_id: None,
        // Follower hooks are never authenticated as incoming hooks.
        token_hash: String::new(),
    };
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::InsertWebhook(webhook.clone()));
    writes.mutate(Mutation::InsertAnnouncementFollower {
        source_channel_id: source,
        target_channel_id: target,
        webhook_id: webhook.id,
    });
    ports.unit_of_work.commit(writes).await?;
    Ok(webhook)
}
