use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::channel_access::ChannelAccess;
use crate::write_set::{Mutation, WriteSet};

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    channel_id: Snowflake,
    status: Option<String>,
) -> AppResult<()> {
    let access = ChannelAccess::visible(ports, channel_id, actor.user_id).await?;
    let guild = access.guild()?;
    if !access.channel.kind.is_voice() {
        return Err(AppError::invalid(
            "status",
            "CHANNEL_TYPE_INVALID",
            "Only voice channels have a status.",
        ));
    }
    if status
        .as_ref()
        .is_some_and(|value| value.chars().count() > 500)
    {
        return Err(AppError::invalid(
            "status",
            "BASE_TYPE_MAX_LENGTH",
            "Status may contain at most 500 characters.",
        ));
    }
    if status.as_ref().is_some_and(|value| value.contains('\0')) {
        return Err(AppError::invalid(
            "status",
            "INVALID_STRING",
            "Status cannot contain a null character.",
        ));
    }
    if !access
        .permissions
        .contains(Permissions::SET_VOICE_CHANNEL_STATUS)
        && !ports
            .presence
            .in_voice_channel(guild.guild.id, channel_id, actor.user_id)
            .await?
    {
        return Err(AppError::missing_permissions());
    }
    let mut channel = access.channel.clone();
    channel.status = status.clone();
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::SetVoiceStatus { channel_id, status })
        .emit(DomainEvent::ChannelUpdate(channel));
    ports.unit_of_work.commit(writes).await
}
