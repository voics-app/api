use domain::channel::ChannelType;
use domain::delivery_mode::DeliveryMode;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;
use crate::support::channel_access::ChannelAccess;
use crate::support::delivery::{effective_mode, viewer_ids};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelAccessInfo {
    pub permissions: Permissions,
    pub guild_id: Option<Snowflake>,
    pub channel_type: ChannelType,
    pub delivery_mode: DeliveryMode,
    pub recipient_ids: Vec<Snowflake>,
}

/// `GET /internal/v1/channels/{id}/access/{user}`: 403 unless the user is a
/// member of the guild or a recipient of the DM.
///
/// `recipient_ids` are the users an end-to-end encrypted or P2P envelope may be addressed to: the
/// DM recipients, or for a non-Standard guild channel the members that can view it (the gateway
/// relay authorises recipients against this list).
pub async fn execute(
    ports: &Ports,
    channel_id: Snowflake,
    user_id: Snowflake,
) -> AppResult<ChannelAccessInfo> {
    let access = ChannelAccess::resolve(ports, channel_id, user_id).await?;
    let delivery_mode = effective_mode(ports, &access.channel).await?;
    let recipient_ids = match (&access.guild, delivery_mode) {
        (Some(guild), mode) if mode != DeliveryMode::Standard => {
            viewer_ids(
                ports,
                &guild.guild,
                &guild.roles,
                &access.channel,
                access.parent.as_ref(),
            )
            .await?
        }
        _ => access.channel.recipient_ids.clone(),
    };
    Ok(ChannelAccessInfo {
        permissions: access.permissions,
        guild_id: access.channel.guild_id,
        channel_type: access.channel.kind,
        delivery_mode,
        recipient_ids,
    })
}
