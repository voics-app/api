use domain::channel::{ChannelType, MAX_GROUP_DM_PARTICIPANTS};
use domain::relationship::RelationshipType;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Denial, Resource};
use crate::events::DomainEvent;
use crate::support::channel_access::ChannelAccess;
use crate::support::system_messages::{self, RECIPIENT_ADD};
use crate::support::views::{private_view, require_user};
use crate::write_set::{Mutation, WriteSet};

/// Any member adds a user to a group DM, up to `MAX_GROUP_DM_PARTICIPANTS` members.
/// Adding an existing member is a no-op.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    channel_id: Snowflake,
    user_id: Snowflake,
) -> AppResult<()> {
    let access = ChannelAccess::visible(ports, channel_id, actor.user_id).await?;
    let mut channel = access.channel;
    if access.guild.is_some() || channel.kind != ChannelType::GroupDm {
        return Err(AppError::invalid(
            "channel_id",
            "CHANNEL_TYPE_INVALID",
            "Recipients can only be added to a group DM.",
        ));
    }
    actor.require_user()?;
    if channel.recipient_ids.contains(&user_id) {
        return Ok(());
    }
    if channel.recipient_ids.len() >= MAX_GROUP_DM_PARTICIPANTS {
        return Err(AppError::invalid(
            "user_id",
            "MAX_GROUP_DM_RECIPIENTS",
            format!("A group DM has at most {MAX_GROUP_DM_PARTICIPANTS} members."),
        ));
    }
    let added = require_user(ports, user_id)
        .await
        .map_err(|_| AppError::NotFound(Resource::User))?;
    let blocked = ports
        .relationships
        .get(user_id, actor.user_id)
        .await?
        .map(|r| r.kind)
        == Some(RelationshipType::Blocked);
    if blocked {
        return Err(AppError::Forbidden(Denial::CannotMessageUser));
    }
    let existing = channel.recipient_ids.clone();
    channel.recipient_ids.push(user_id);
    channel.recipient_ids.sort();
    let view = private_view(ports, channel.clone()).await?;
    let mut others = view.clone();
    others.recipients.retain(|u| u.id != user_id);
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpdateChannel(channel.clone()))
        .emit(DomainEvent::PrivateChannelCreateFor {
            view: view.clone(),
            user_id,
        })
        .emit(DomainEvent::PrivateChannelUpdate(others))
        .emit(DomainEvent::RecipientAdd {
            channel_id,
            user: added,
            notify: existing,
        });
    ports.unit_of_work.commit(writes).await?;
    system_messages::post(
        ports,
        view.delivery_mode,
        channel_id,
        actor.user_id,
        RECIPIENT_ADD,
        Some(user_id),
        None,
    )
    .await;
    Ok(())
}
