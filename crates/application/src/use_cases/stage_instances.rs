//! Lifecycle of a live stage instance. State and gateway events commit together.
use domain::channel::ChannelType;
use domain::permissions::Permissions;
use domain::stage_instance::StageInstance;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::events::DomainEvent;
use crate::support::channel_access::ChannelAccess;
use crate::write_set::{Mutation, WriteSet};

async fn access(
    ports: &Ports,
    actor: &Actor,
    channel_id: Snowflake,
    manage: bool,
) -> AppResult<ChannelAccess> {
    let access = ChannelAccess::visible(ports, channel_id, actor.user_id).await?;
    if access.channel.kind != ChannelType::GuildStageVoice || access.channel.guild_id.is_none() {
        return Err(AppError::invalid(
            "channel_id",
            "INVALID_CHANNEL_TYPE",
            "A guild stage channel is required.",
        ));
    }
    if manage {
        access.require(Permissions::MANAGE_CHANNELS)?;
    }
    Ok(access)
}

fn validate_topic(topic: &str) -> AppResult<()> {
    if topic.trim().is_empty() || topic.chars().count() > 120 {
        return Err(AppError::invalid(
            "topic",
            "INVALID_TOPIC",
            "Topic must contain 1 to 120 characters.",
        ));
    }
    Ok(())
}

pub async fn get(ports: &Ports, actor: &Actor, channel_id: Snowflake) -> AppResult<StageInstance> {
    access(ports, actor, channel_id, false).await?;
    ports
        .channels
        .stage_instance(channel_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Channel))
}

pub async fn start(
    ports: &Ports,
    actor: &Actor,
    channel_id: Snowflake,
    topic: String,
) -> AppResult<StageInstance> {
    let access = access(ports, actor, channel_id, true).await?;
    validate_topic(&topic)?;
    let stage = StageInstance {
        id: ports.ids.next_id()?,
        guild_id: access.channel.guild_id.expect("validated guild stage"),
        channel_id,
        topic,
    };
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::InsertStageInstance(stage.clone()))
        .emit(DomainEvent::StageInstanceCreate(stage.clone()));
    ports.unit_of_work.commit(writes).await?;
    Ok(stage)
}

pub async fn update(
    ports: &Ports,
    actor: &Actor,
    channel_id: Snowflake,
    topic: String,
) -> AppResult<StageInstance> {
    access(ports, actor, channel_id, true).await?;
    validate_topic(&topic)?;
    let mut stage = ports
        .channels
        .stage_instance(channel_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Channel))?;
    stage.topic = topic;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpdateStageInstance(stage.clone()))
        .emit(DomainEvent::StageInstanceUpdate(stage.clone()));
    ports.unit_of_work.commit(writes).await?;
    Ok(stage)
}

pub async fn end(ports: &Ports, actor: &Actor, channel_id: Snowflake) -> AppResult<()> {
    access(ports, actor, channel_id, true).await?;
    let stage = ports
        .channels
        .stage_instance(channel_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Channel))?;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::DeleteStageInstance {
            channel_id,
            id: stage.id,
        })
        .emit(DomainEvent::StageInstanceDelete(stage));
    ports.unit_of_work.commit(writes).await
}
