use domain::channel::Channel;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::events::DomainEvent;
use crate::support::guild_context::GuildContext;
use crate::use_cases::channels::channel_rules::apply_parent;
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone)]
pub struct ChannelPosition {
    pub id: Snowflake,
    pub position: Option<i32>,
    /// `Some(None)` moves the channel out of its category.
    pub parent_id: Option<Option<Snowflake>>,
    pub lock_permissions: bool,
}

/// `PATCH /guilds/{id}/channels`.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    moves: Vec<ChannelPosition>,
) -> AppResult<()> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_CHANNELS)?;
    let channels = ports.channels.list_for_guild(guild_id).await?;
    let mut writes = WriteSet::new();
    for change in moves {
        let mut channel: Channel = channels
            .iter()
            .find(|c| c.id == change.id)
            .cloned()
            .ok_or(AppError::NotFound(Resource::Channel))?;
        if let Some(position) = change.position {
            channel.position = position;
        }
        if let Some(parent_id) = change.parent_id {
            apply_parent(ports, &mut channel, parent_id).await?;
            if change.lock_permissions {
                if let Some(parent) = channels.iter().find(|c| Some(c.id) == parent_id) {
                    channel.overwrites = parent.overwrites.clone();
                }
            }
        }
        writes
            .mutate(Mutation::UpdateChannel(channel.clone()))
            .emit(DomainEvent::ChannelUpdate(channel));
    }
    if writes.is_empty() {
        return Ok(());
    }
    ports.unit_of_work.commit(writes).await
}
