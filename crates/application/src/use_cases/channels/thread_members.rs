use domain::channel::ChannelType;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::channel_access::ChannelAccess;
use crate::write_set::{Mutation, WriteSet};

/// Join a public thread, or invite an eligible guild member to a private thread.
pub async fn join(
    ports: &Ports,
    actor: &Actor,
    thread_id: Snowflake,
    user_id: Snowflake,
) -> AppResult<()> {
    let access = ChannelAccess::visible(ports, thread_id, actor.user_id).await?;
    let thread = &access.channel;
    if !thread.kind.is_thread() {
        return Err(AppError::invalid(
            "channel_id",
            "CHANNEL_TYPE_INVALID",
            "Expected a thread.",
        ));
    }
    let guild = access.guild()?;
    let manages = access.permissions.contains(Permissions::MANAGE_THREADS);
    if user_id != actor.user_id {
        let may_invite = thread.kind == ChannelType::PrivateThread
            && thread
                .thread_metadata
                .as_ref()
                .is_some_and(|meta| meta.invitable);
        if !manages && !may_invite {
            return Err(AppError::missing_permissions());
        }
    }
    if thread
        .thread_metadata
        .as_ref()
        .is_some_and(|meta| meta.locked)
        && !manages
    {
        return Err(AppError::missing_permissions());
    }
    let parent_id = thread.parent_id.ok_or_else(AppError::missing_access)?;
    ChannelAccess::visible(ports, parent_id, user_id).await?;
    if thread.thread_member_ids.contains(&user_id) {
        return Ok(());
    }
    let now = ports.clock.now();
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::SetThreadMember {
            thread_id,
            guild_id: guild.guild.id,
            user_id,
            joined_at: Some(now),
        })
        .emit(DomainEvent::ThreadMembersUpdate {
            guild_id: guild.guild.id,
            thread_id,
            user_id,
            joined_at: Some(now),
            member_count: thread.thread_member_ids.len() + 1,
            version: 0,
            member_ids: Vec::new(),
        });
    ports.unit_of_work.commit(writes).await
}

/// Leave a thread, or remove another member with MANAGE_THREADS.
pub async fn leave(
    ports: &Ports,
    actor: &Actor,
    thread_id: Snowflake,
    user_id: Snowflake,
) -> AppResult<()> {
    let access = ChannelAccess::visible(ports, thread_id, actor.user_id).await?;
    let thread = &access.channel;
    if !thread.kind.is_thread() {
        return Err(AppError::invalid(
            "channel_id",
            "CHANNEL_TYPE_INVALID",
            "Expected a thread.",
        ));
    }
    let guild = access.guild()?;
    if user_id != actor.user_id {
        access.require(Permissions::MANAGE_THREADS)?;
    }
    if !thread.thread_member_ids.contains(&user_id) {
        return Ok(());
    }
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::SetThreadMember {
            thread_id,
            guild_id: guild.guild.id,
            user_id,
            joined_at: None,
        })
        .emit(DomainEvent::ThreadMembersUpdate {
            guild_id: guild.guild.id,
            thread_id,
            user_id,
            joined_at: None,
            member_count: thread.thread_member_ids.len() - 1,
            version: 0,
            member_ids: Vec::new(),
        });
    ports.unit_of_work.commit(writes).await
}
