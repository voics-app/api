use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::support::channel_access::ChannelAccess;
use domain::channel::{Channel, ChannelType};
use domain::permissions::Permissions;
use domain::Snowflake;

/// Lists visible, published posts of one forum.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    parent_id: Snowflake,
) -> AppResult<Vec<Channel>> {
    let access = ChannelAccess::visible(ports, parent_id, actor.user_id).await?;
    let parent = &access.channel;
    if parent.kind != ChannelType::GuildForum {
        return Err(AppError::invalid(
            "channel_id",
            "CHANNEL_TYPE_INVALID",
            "Expected a forum channel.",
        ));
    }
    let ctx = access.guild()?;
    let channels = ports.channels.list_for_guild(ctx.guild.id).await?;
    Ok(channels
        .into_iter()
        .filter(|channel| {
            channel.parent_id == Some(parent_id)
                && channel.kind == ChannelType::PublicThread
                && channel.delivery_mode == parent.delivery_mode
                && !channel
                    .forum_settings
                    .get("pending")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                && ctx
                    .channel_permissions(channel, Some(parent))
                    .contains(Permissions::VIEW_CHANNEL)
        })
        .collect())
}
