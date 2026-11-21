use domain::channel::Channel;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::support::channel_access::ChannelAccess;

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    parent_id: Snowflake,
    archived: bool,
) -> AppResult<Vec<Channel>> {
    let access = ChannelAccess::visible(ports, parent_id, actor.user_id).await?;
    if !access.channel.kind.supports_threads() {
        return Err(AppError::invalid(
            "channel_id",
            "CHANNEL_TYPE_INVALID",
            "Expected a thread parent.",
        ));
    }
    let guild = access.guild()?;
    let channels = ports.channels.list_for_guild(guild.guild.id).await?;
    Ok(channels
        .into_iter()
        .filter(|channel| channel.kind.is_thread() && channel.parent_id == Some(parent_id))
        .filter(|channel| {
            channel
                .thread_metadata
                .as_ref()
                .is_some_and(|meta| meta.archived == archived)
        })
        .filter(|channel| {
            guild
                .channel_permissions(channel, Some(&access.channel))
                .contains(Permissions::VIEW_CHANNEL)
        })
        .collect())
}
