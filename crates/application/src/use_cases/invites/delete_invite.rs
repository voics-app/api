use serde_json::json;

use domain::audit::{AuditAction, AuditChange};
use domain::permissions::Permissions;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::channel_access::ChannelAccess;
use crate::use_cases::invites::get_invite::{view, InviteView};
use crate::write_set::{Mutation, WriteSet};

/// Allowed for the inviter, MANAGE_CHANNELS on the channel or MANAGE_GUILD.
pub async fn execute(ports: &Ports, actor: &Actor, code: &str, audit: AuditContext<'_>) -> AppResult<InviteView> {
    let invite = ports.invites.find(code).await?.ok_or(AppError::NotFound(Resource::Invite))?;
    let access = ChannelAccess::resolve(ports, invite.channel_id, actor.user_id).await?;
    let ctx = access.guild()?;
    let allowed = invite.inviter_id == Some(actor.user_id)
        || access.permissions.contains(Permissions::MANAGE_CHANNELS)
        || ctx.permissions().contains(Permissions::MANAGE_GUILD);
    if !allowed {
        return Err(AppError::missing_permissions());
    }
    let result = view(ports, invite.clone()).await?;
    let changes = vec![AuditChange::new("code", Some(json!(invite.code)), None)];
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::DeleteInvite(invite.code.clone()))
        .emit(DomainEvent::InviteDelete { guild_id: invite.guild_id, channel_id: invite.channel_id, code: invite.code })
        .audit(audit.entry(ports, invite.guild_id, AuditAction::InviteDelete, None, changes)?);
    ports.unit_of_work.commit(writes).await?;
    Ok(result)
}
