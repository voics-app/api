use domain::audit::AuditAction;
use domain::channel::ChannelType;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::guild_context::GuildContext;
use crate::support::views::require_user;
use crate::write_set::{Mutation, WriteSet};

/// The longest purge window Discord offers: seven days.
pub const MAX_DELETE_MESSAGE_SECONDS: u32 = 604_800;

/// Bans a user (member or not) and removes their membership. When
/// `delete_message_seconds` is positive, asks the messages service (through a
/// bus event) to purge the user's messages from that long ago in every
/// channel of the guild.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    user_id: Snowflake,
    delete_message_seconds: Option<u32>,
    audit: AuditContext<'_>,
) -> AppResult<()> {
    let seconds = delete_message_seconds.unwrap_or(0);
    if seconds > MAX_DELETE_MESSAGE_SECONDS {
        return Err(AppError::invalid(
            "delete_message_seconds",
            "INVALID_BODY",
            format!("must be between 0 and {MAX_DELETE_MESSAGE_SECONDS}"),
        ));
    }
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::BAN_MEMBERS)?;
    if user_id == actor.user_id || user_id == ctx.guild.owner_id {
        return Err(AppError::missing_permissions());
    }
    let user = require_user(ports, user_id).await?;
    let member = ports.guilds.member(guild_id, user_id).await?;
    if let Some(target) = &member {
        ctx.require_outranks(target)?;
    }
    if ports.guilds.ban(guild_id, user_id).await?.is_some() {
        return Ok(());
    }
    let mut writes = WriteSet::new();
    if member.is_some() {
        writes
            .mutate(Mutation::DeleteMember { guild_id, user_id })
            .emit(DomainEvent::MemberRemove {
                guild_id,
                user: user.clone(),
            })
            .emit(DomainEvent::GuildDelete {
                guild_id,
                recipients: Some(vec![user_id]),
            });
    }
    if seconds > 0 {
        let channel_ids: Vec<Snowflake> = ports
            .channels
            .list_for_guild(guild_id)
            .await?
            .into_iter()
            .filter(|channel| channel.kind != ChannelType::GuildCategory)
            .map(|channel| channel.id)
            .collect();
        if !channel_ids.is_empty() {
            let now_ms = ports.clock.now().timestamp_millis().max(0) as u64;
            writes.emit(DomainEvent::BanMessagePurge {
                guild_id,
                user_id,
                channel_ids,
                since_ms: now_ms.saturating_sub(u64::from(seconds) * 1000),
                delete_message_seconds: seconds,
            });
        }
    }
    writes
        .mutate(Mutation::InsertBan {
            guild_id,
            user_id,
            reason: audit.reason.map(str::to_owned),
        })
        .emit(DomainEvent::BanAdd { guild_id, user })
        .audit(audit.entry(
            ports,
            guild_id,
            AuditAction::MemberBanAdd,
            Some(user_id),
            vec![],
        )?);
    ports.unit_of_work.commit(writes).await
}
