use serde_json::json;

use domain::audit::{AuditAction, AuditChange};
use domain::invite::{Invite, MAX_INVITE_AGE_SECONDS, MAX_INVITE_USES};
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::channel_access::ChannelAccess;
use crate::support::views::require_user;
use crate::use_cases::invites::get_invite::{view, InviteView};
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone, Default)]
pub struct CreateInviteInput {
    pub max_age: Option<u32>,
    pub max_uses: Option<u32>,
    pub temporary: Option<bool>,
}

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    channel_id: Snowflake,
    input: CreateInviteInput,
    audit: AuditContext<'_>,
) -> AppResult<InviteView> {
    let access = ChannelAccess::visible(ports, channel_id, actor.user_id).await?;
    let ctx = access.guild()?;
    access.require(Permissions::CREATE_INSTANT_INVITE)?;
    if access.channel.kind.is_thread() {
        return Err(AppError::invalid(
            "channel_id",
            "CHANNEL_TYPE_INVALID",
            "Invites cannot target threads.",
        ));
    }
    let max_age = input.max_age.unwrap_or(86_400);
    if max_age > MAX_INVITE_AGE_SECONDS {
        return Err(AppError::invalid(
            "max_age",
            "BASE_TYPE_BAD_INT",
            "Must be between 0 and 604800.",
        ));
    }
    let max_uses = input.max_uses.unwrap_or(0);
    if max_uses > MAX_INVITE_USES {
        return Err(AppError::invalid(
            "max_uses",
            "BASE_TYPE_BAD_INT",
            "Must be between 0 and 100.",
        ));
    }
    let invite = Invite {
        code: ports.secrets.invite_code(),
        guild_id: ctx.guild.id,
        channel_id,
        inviter_id: Some(actor.user_id),
        max_age,
        max_uses,
        uses: 0,
        temporary: input.temporary.unwrap_or(false),
        created_at: ports.clock.now(),
    };
    let inviter = require_user(ports, actor.user_id).await?;
    let changes = vec![
        AuditChange::new("code", None, Some(json!(invite.code))),
        AuditChange::new("max_age", None, Some(json!(max_age))),
        AuditChange::new("max_uses", None, Some(json!(max_uses))),
    ];
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::InsertInvite(invite.clone()))
        .emit(DomainEvent::InviteCreate {
            invite: invite.clone(),
            inviter: Some(inviter),
        })
        .audit(audit.entry(
            ports,
            ctx.guild.id,
            AuditAction::InviteCreate,
            None,
            changes,
        )?);
    ports.unit_of_work.commit(writes).await?;
    view(ports, invite).await
}
