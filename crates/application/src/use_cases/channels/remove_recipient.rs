use domain::channel::{Channel, ChannelType};
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::events::{ChannelView, DomainEvent};
use crate::support::channel_access::ChannelAccess;
use crate::support::system_messages::{self, RECIPIENT_REMOVE};
use crate::support::views::{private_view, require_user};
use crate::write_set::{Mutation, WriteSet};

/// The owner removes `user_id`; any member removes themself (leave). `None` means the caller.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    channel_id: Snowflake,
    user_id: Option<Snowflake>,
) -> AppResult<()> {
    let access = ChannelAccess::visible(ports, channel_id, actor.user_id).await?;
    if access.guild.is_some() || access.channel.kind != ChannelType::GroupDm {
        return Err(AppError::invalid(
            "channel_id",
            "CHANNEL_TYPE_INVALID",
            "Recipients can only be removed from a group DM.",
        ));
    }
    let target = user_id.unwrap_or(actor.user_id);
    if target != actor.user_id && access.channel.owner_id != Some(actor.user_id) {
        return Err(AppError::missing_permissions());
    }
    if !access.channel.recipient_ids.contains(&target) {
        return Err(AppError::NotFound(Resource::User));
    }
    remove(ports, actor, access.channel, target).await?;
    Ok(())
}

/// Removes `target` from the group: ownership moves to a remaining member when the owner goes,
/// the group is deleted when nobody is left, and the removed user's mode override is dropped.
pub(crate) async fn remove(
    ports: &Ports,
    actor: &Actor,
    mut channel: Channel,
    target: Snowflake,
) -> AppResult<ChannelView> {
    let leaving = require_user(ports, target).await?;
    let before = private_view(ports, channel.clone()).await?;
    // Posted first: the author must still be a member for the messages service to accept it.
    system_messages::post(
        ports,
        before.delivery_mode,
        channel.id,
        actor.user_id,
        RECIPIENT_REMOVE,
        Some(target),
        None,
    )
    .await;
    channel.recipient_ids.retain(|id| *id != target);
    if channel.owner_id == Some(target) {
        channel.owner_id = channel.recipient_ids.first().copied();
    }
    let mut writes = WriteSet::new();
    writes.emit(DomainEvent::PrivateChannelDelete {
        channel: channel.clone(),
        user_id: target,
    });
    if channel.recipient_ids.is_empty() {
        writes.mutate(Mutation::DeleteChannel(channel.id));
    } else {
        let remaining = private_view(ports, channel.clone()).await?;
        writes
            .mutate(Mutation::UpdateChannel(channel.clone()))
            .emit(DomainEvent::PrivateChannelUpdate(remaining))
            .emit(DomainEvent::RecipientRemove {
                channel_id: channel.id,
                user: leaving,
                notify: channel.recipient_ids.clone(),
            });
    }
    writes.mutate(Mutation::SetDeliveryOverride {
        user_id: target,
        channel_id: channel.id,
        mode: None,
    });
    ports.unit_of_work.commit(writes).await?;
    Ok(ChannelView::Private(private_view(ports, channel).await?))
}
