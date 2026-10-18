use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::events::ChannelView;
use crate::support::channel_access::ChannelAccess;
use crate::support::views::private_view;

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    channel_id: Snowflake,
) -> AppResult<ChannelView> {
    let access = ChannelAccess::visible(ports, channel_id, actor.user_id).await?;
    if access.guild.is_some() {
        return Ok(ChannelView::Guild(access.channel));
    }
    Ok(ChannelView::Private(
        private_view(ports, access.channel).await?,
    ))
}
