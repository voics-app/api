use domain::channel::{Channel, ChannelType};
use domain::relationship::RelationshipType;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Denial, Resource};
use crate::events::{DomainEvent, PrivateChannelView};
use crate::support::views::private_view;
use crate::write_set::{Mutation, WriteSet};

/// One recipient opens (or reuses) a DM; several create a group DM.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    recipient_ids: Vec<Snowflake>,
) -> AppResult<PrivateChannelView> {
    let mut others: Vec<Snowflake> = recipient_ids
        .into_iter()
        .filter(|id| *id != actor.user_id)
        .collect();
    others.sort();
    others.dedup();
    let max = ports.settings.group_dm_max_participants;
    if others.is_empty() || others.len() + 1 > max {
        return Err(AppError::invalid(
            "recipient_ids",
            "BASE_TYPE_BAD_LENGTH",
            format!("Must contain between 1 and {} other users.", max - 1),
        ));
    }
    if others.len() > 1 {
        actor.require_user()?;
    }
    let users = ports.users.find_many(&others).await?;
    if users.len() != others.len() {
        return Err(AppError::NotFound(Resource::User));
    }
    for other in &others {
        let blocked = ports
            .relationships
            .get(*other, actor.user_id)
            .await?
            .map(|r| r.kind)
            == Some(RelationshipType::Blocked);
        if blocked {
            return Err(AppError::Forbidden(Denial::CannotMessageUser));
        }
    }

    if let [other] = others.as_slice() {
        if let Some(existing) = ports.channels.find_dm(actor.user_id, *other).await? {
            return private_view(ports, existing).await;
        }
    }

    let kind = if others.len() == 1 {
        ChannelType::Dm
    } else {
        ChannelType::GroupDm
    };
    let mut channel = Channel::new(ports.ids.next_id()?, kind, None, ports.clock.now());
    channel.recipient_ids = std::iter::once(actor.user_id).chain(others).collect();
    if kind == ChannelType::GroupDm {
        channel.owner_id = Some(actor.user_id);
    }
    let view = private_view(ports, channel.clone()).await?;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::InsertChannel(channel))
        .emit(DomainEvent::PrivateChannelCreate(view.clone()));
    ports.unit_of_work.commit(writes).await?;
    Ok(view)
}
