use std::collections::HashMap;

use domain::access::GuildAccess;
use domain::channel::Channel;
use domain::delivery_mode::{resolve, ChannelDelivery, DeliveryMode, ParticipantPreference};
use domain::guild::{Guild, Role};
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;

/// The effective delivery mode of any channel (P11).
pub async fn effective_mode(ports: &Ports, channel: &Channel) -> AppResult<DeliveryMode> {
    effective_mode_with(ports, channel, None).await
}

/// Effective mode as it will be once `pending` (user, override) is stored.
pub async fn effective_mode_with(
    ports: &Ports,
    channel: &Channel,
    pending: Option<(Snowflake, Option<DeliveryMode>)>,
) -> AppResult<DeliveryMode> {
    if channel.guild_id.is_some() {
        return Ok(resolve(ChannelDelivery::Guild {
            configured: channel.delivery_mode,
        }));
    }
    let mut overrides: HashMap<_, _> = ports
        .channels
        .delivery_overrides(channel.id)
        .await?
        .into_iter()
        .collect();
    if let Some((user_id, mode)) = pending {
        match mode {
            Some(mode) => overrides.insert(user_id, mode),
            None => overrides.remove(&user_id),
        };
    }
    let settings = ports.users.settings_many(&channel.recipient_ids).await?;
    let participants: Vec<ParticipantPreference> = channel
        .recipient_ids
        .iter()
        .map(|user_id| ParticipantPreference {
            chat_override: overrides.get(user_id).copied(),
            account_default: settings
                .iter()
                .find(|s| s.user_id == *user_id)
                .map(|s| s.default_delivery_mode)
                .unwrap_or_default(),
        })
        .collect();
    Ok(resolve(ChannelDelivery::Private {
        participants: &participants,
    }))
}

/// How many guild members can view `channel` (for the ephemeral audience limit).
pub async fn viewer_count(
    ports: &Ports,
    guild: &Guild,
    roles: &[Role],
    channel: &Channel,
    parent: Option<&Channel>,
) -> AppResult<usize> {
    let members = ports.guilds.members(guild.id).await?;
    let access = GuildAccess::new(guild, roles, ports.clock.now());
    Ok(members
        .iter()
        .filter(|m| {
            access
                .in_channel(m, channel, parent)
                .contains(Permissions::VIEW_CHANNEL)
        })
        .count())
}
