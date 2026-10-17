use domain::delivery_mode::{validate_private_override, DeliveryMode};
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::events::{DomainEvent, PrivateChannelView};
use crate::support::delivery::effective_mode_with;
use crate::write_set::{Mutation, WriteSet};

/// Per-chat override for DMs and group DMs (`None` clears it). Guild
/// channels are configured by MANAGE_CHANNELS instead (400).
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    channel_id: Snowflake,
    mode: Option<DeliveryMode>,
) -> AppResult<PrivateChannelView> {
    actor.require_user()?;
    let channel = ports.channels.find(channel_id).await?.ok_or(AppError::NotFound(Resource::Channel))?;
    if channel.guild_id.is_some() {
        return Err(AppError::invalid(
            "channel_id",
            "DELIVERY_MODE_GUILD_CHANNEL",
            "Guild channel delivery modes are set by members with Manage Channels.",
        ));
    }
    if !channel.is_recipient(actor.user_id) {
        return Err(AppError::missing_access());
    }
    if let Some(mode) = mode {
        validate_private_override(mode, channel.recipient_ids.len())?;
    }
    let delivery_mode = effective_mode_with(ports, &channel, Some((actor.user_id, mode))).await?;
    let recipients = ports.users.find_many(&channel.recipient_ids).await?;
    let view = PrivateChannelView { channel, recipients, delivery_mode };
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::SetDeliveryOverride { user_id: actor.user_id, channel_id, mode })
        .emit(DomainEvent::PrivateChannelUpdate(view.clone()));
    ports.unit_of_work.commit(writes).await?;
    Ok(view)
}
