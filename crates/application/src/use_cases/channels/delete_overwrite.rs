use domain::audit::AuditAction;
use domain::delivery_mode::DeliveryMode;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::channel_access::ChannelAccess;
use crate::use_cases::channels::channel_rules::check_delivery_mode;
use crate::write_set::{Mutation, WriteSet};

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    channel_id: Snowflake,
    overwrite_id: Snowflake,
    audit: AuditContext<'_>,
) -> AppResult<()> {
    let access = ChannelAccess::visible(ports, channel_id, actor.user_id).await?;
    let ctx = access.guild()?;
    access.require(Permissions::MANAGE_ROLES)?;
    let mut channel = access.channel.clone();
    let before = channel.overwrites.len();
    channel.overwrites.retain(|o| o.id != overwrite_id);
    if channel.overwrites.len() == before {
        return Err(AppError::NotFound(Resource::Overwrite));
    }
    if channel.delivery_mode == DeliveryMode::Ephemeral {
        check_delivery_mode(ports, ctx, &channel, DeliveryMode::Ephemeral).await?;
    }
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpdateChannel(channel.clone()))
        .emit(DomainEvent::ChannelUpdate(channel.clone()))
        .audit(audit.entry(ports, ctx.guild.id, AuditAction::ChannelOverwriteDelete, Some(channel.id), vec![])?);
    ports.unit_of_work.commit(writes).await
}
