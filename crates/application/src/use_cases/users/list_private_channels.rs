use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::events::PrivateChannelView;
use crate::support::views::private_view;

pub async fn execute(ports: &Ports, actor: &Actor) -> AppResult<Vec<PrivateChannelView>> {
    let channels = ports.channels.list_private(actor.user_id).await?;
    let mut views = Vec::with_capacity(channels.len());
    for channel in channels {
        views.push(private_view(ports, channel).await?);
    }
    Ok(views)
}
