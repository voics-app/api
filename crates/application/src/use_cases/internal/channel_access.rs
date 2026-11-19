use domain::channel::ChannelType;
use domain::delivery_mode::DeliveryMode;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::{AppError, AppResult};
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
    ensure_current_account_access(ports, user_id).await?;
    let access = ChannelAccess::resolve(ports, channel_id, user_id).await?;
    if access
        .channel
        .forum_settings
        .get("status")
        .and_then(serde_json::Value::as_str)
        == Some("deleted")
    {
        return Err(AppError::missing_access());
    }
    if access
        .channel
        .forum_settings
        .get("pending")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        && access.channel.owner_id != Some(user_id)
    {
        return Err(AppError::missing_access());
    }
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

/// Offline work has no live user session to revoke. Check the account itself
/// on each service lookup, including guild owners and private-channel recipients.
async fn ensure_current_account_access(ports: &Ports, user_id: Snowflake) -> AppResult<()> {
    let user = ports
        .users
        .find(user_id)
        .await?
        .ok_or_else(AppError::missing_access)?;
    if user.is_suspended(ports.clock.now())
        || ports.users.account_state(user_id).await?.is_inactive()
    {
        return Err(AppError::missing_access());
    }
    Ok(())
}
