use serde_json::json;

use domain::audit::{AuditAction, AuditChange};
use domain::channel::{Channel, ChannelType};
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::{ChannelView, DomainEvent};
use crate::support::audit::AuditContext;
use crate::support::channel_access::ChannelAccess;
use crate::support::views::{private_view, require_user};
use crate::write_set::{Mutation, WriteSet};

/// Deletes a guild channel or thread; for group DMs the caller leaves;
/// DMs are simply closed for the caller.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    channel_id: Snowflake,
    audit: AuditContext<'_>,
) -> AppResult<ChannelView> {
    let access = ChannelAccess::visible(ports, channel_id, actor.user_id).await?;
    let channel = access.channel.clone();
    let Some(ctx) = &access.guild else {
        return leave_private(ports, actor, channel).await;
    };
    let is_thread = channel.kind.is_thread();
    let allowed = if is_thread {
        access.permissions.contains(Permissions::MANAGE_THREADS)
    } else {
        access.permissions.contains(Permissions::MANAGE_CHANNELS)
    };
    if !allowed {
        return Err(AppError::missing_permissions());
    }
    let (event, action) = if is_thread {
        (DomainEvent::ThreadDelete(channel.clone()), AuditAction::ThreadDelete)
    } else {
        (DomainEvent::ChannelDelete(channel.clone()), AuditAction::ChannelDelete)
    };
    let changes = vec![AuditChange::new("name", Some(json!(channel.name)), None)];
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::DeleteChannel(channel.id))
        .emit(event)
        .audit(audit.entry(ports, ctx.guild.id, action, Some(channel.id), changes)?);
    ports.unit_of_work.commit(writes).await?;
    Ok(ChannelView::Guild(channel))
}

async fn leave_private(ports: &Ports, actor: &Actor, mut channel: Channel) -> AppResult<ChannelView> {
    if channel.kind != ChannelType::GroupDm {
        return Ok(ChannelView::Private(private_view(ports, channel).await?));
    }
    let leaver = require_user(ports, actor.user_id).await?;
    channel.recipient_ids.retain(|id| *id != actor.user_id);
    if channel.owner_id == Some(actor.user_id) {
        channel.owner_id = channel.recipient_ids.first().copied();
    }
    let mut writes = WriteSet::new();
    writes.emit(DomainEvent::PrivateChannelDelete { channel: channel.clone(), user_id: actor.user_id });
    if channel.recipient_ids.is_empty() {
        writes.mutate(Mutation::DeleteChannel(channel.id));
    } else {
        writes.mutate(Mutation::UpdateChannel(channel.clone())).emit(DomainEvent::RecipientRemove {
            channel_id: channel.id,
            user: leaver,
            notify: channel.recipient_ids.clone(),
        });
    }
    writes.mutate(Mutation::SetDeliveryOverride { user_id: actor.user_id, channel_id: channel.id, mode: None });
    ports.unit_of_work.commit(writes).await?;
    Ok(ChannelView::Private(private_view(ports, channel).await?))
}
