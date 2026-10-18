//! `GET /internal/v1/channels/{id}/delivery`: where a channel lives and how
//! it delivers, without a user. messages needs it for webhook posts, since a
//! webhook is not a guild member and has no access to resolve.

use domain::channel::ChannelType;
use domain::delivery_mode::DeliveryMode;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::{AppError, AppResult, Resource};
use crate::support::delivery::effective_mode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRouting {
    pub guild_id: Option<Snowflake>,
    pub channel_type: ChannelType,
    pub delivery_mode: DeliveryMode,
    pub recipient_ids: Vec<Snowflake>,
}

pub async fn execute(ports: &Ports, channel_id: Snowflake) -> AppResult<ChannelRouting> {
    let channel = ports
        .channels
        .find(channel_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Channel))?;
    let delivery_mode = effective_mode(ports, &channel).await?;
    Ok(ChannelRouting {
        guild_id: channel.guild_id,
        channel_type: channel.kind,
        delivery_mode,
        recipient_ids: channel.recipient_ids.clone(),
    })
}
