use serde_json::json;

use domain::audit::{AuditAction, AuditChange};
use domain::delivery_mode::DeliveryMode;
use domain::permissions::{OverwriteKind, PermissionOverwrite};
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::channel_access::ChannelAccess;
use crate::use_cases::channels::channel_rules::{check_delivery_mode, check_overwrites};
use crate::write_set::{Mutation, WriteSet};

/// `PUT /channels/{id}/permissions/{overwrite_id}`.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    channel_id: Snowflake,
    overwrite: PermissionOverwrite,
    audit: AuditContext<'_>,
) -> AppResult<()> {
    let access = ChannelAccess::visible(ports, channel_id, actor.user_id).await?;
    let ctx = access.guild()?;
    if access.channel.kind.is_thread() {
        return Err(AppError::invalid(
            "channel_id",
            "CHANNEL_TYPE_INVALID",
            "Threads inherit overwrites.",
        ));
    }
    check_overwrites(ctx, access.permissions, std::slice::from_ref(&overwrite))?;
    if overwrite.kind == OverwriteKind::Member
        && ports
            .guilds
            .member(ctx.guild.id, overwrite.id)
            .await?
            .is_none()
    {
        return Err(AppError::NotFound(Resource::Member));
    }
    let mut channel = access.channel.clone();
    let existed = channel.overwrites.iter().any(|o| o.id == overwrite.id);
    channel.overwrites.retain(|o| o.id != overwrite.id);
    channel.overwrites.push(overwrite);
    if channel.delivery_mode == DeliveryMode::Ephemeral {
        check_delivery_mode(ports, ctx, &channel, DeliveryMode::Ephemeral).await?;
    }
    let action = if existed {
        AuditAction::ChannelOverwriteUpdate
    } else {
        AuditAction::ChannelOverwriteCreate
    };
    let changes = vec![
        AuditChange::new(
            "allow",
            None,
            Some(json!(overwrite.allow.bits().to_string())),
        ),
        AuditChange::new("deny", None, Some(json!(overwrite.deny.bits().to_string()))),
    ];
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpdateChannel(channel.clone()))
        .emit(DomainEvent::ChannelUpdate(channel.clone()))
        .audit(audit.entry(ports, ctx.guild.id, action, Some(channel.id), changes)?);
    ports.unit_of_work.commit(writes).await
}
