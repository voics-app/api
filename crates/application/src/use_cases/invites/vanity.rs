use domain::access::GuildAccess;
use domain::audit::{AuditAction, AuditChange};
use domain::channel::{Channel, ChannelType};
use domain::guild::Member;
use domain::invite::Invite;
use domain::permissions::Permissions;
use domain::vanity::{normalize_code, VanityInvite};
use domain::Snowflake;
use serde_json::json;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::support::audit::AuditContext;
use crate::support::guild_context::GuildContext;
use crate::write_set::{Mutation, WriteSet};

pub async fn get(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
) -> AppResult<Option<VanityInvite>> {
    GuildContext::load(ports, guild_id, actor.user_id)
        .await?
        .require(Permissions::MANAGE_GUILD)?;
    ports.invites.vanity_for_guild(guild_id).await
}

pub async fn update(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    code: Option<&str>,
    audit: AuditContext<'_>,
) -> AppResult<Option<VanityInvite>> {
    let before = get(ports, actor, guild_id).await?;
    let code = code.map(normalize_code).transpose()?;
    if code.is_some() && public_channel(ports, guild_id).await?.is_none() {
        return Err(AppError::invalid(
            "code",
            "VANITY_CHANNEL_REQUIRED",
            "A text channel visible to everyone is required.",
        ));
    }
    if before.as_ref().map(|v| &v.code) == code.as_ref() {
        return Ok(before);
    }
    let invite = code.map(|code| VanityInvite {
        guild_id,
        code,
        uses: 0,
        created_at: ports.clock.now(),
    });
    let change = AuditChange::new(
        "vanity_url_code",
        Some(json!(before.as_ref().map(|v| &v.code))),
        Some(json!(invite.as_ref().map(|v| &v.code))),
    );
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::SetVanityInvite {
            guild_id,
            invite: invite.clone(),
        })
        .audit(audit.entry(
            ports,
            guild_id,
            AuditAction::GuildUpdate,
            Some(guild_id),
            vec![change],
        )?);
    ports.unit_of_work.commit(writes).await?;
    Ok(invite)
}

/// Only the base @everyone permissions are used, so the preview never names a
/// private channel even when its creator is an administrator.
async fn public_channel(ports: &Ports, guild_id: Snowflake) -> AppResult<Option<Channel>> {
    let guild = ports
        .guilds
        .find(guild_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Guild))?;
    let roles = ports.guilds.roles(guild_id).await?;
    let access = GuildAccess::new(&guild, &roles, ports.clock.now());
    let visitor = Member::new(guild_id, Snowflake::new(0), ports.clock.now());
    let mut channels = ports.channels.list_for_guild(guild_id).await?;
    channels.sort_by_key(|c| (c.position, c.id));
    Ok(channels.into_iter().find(|c| {
        c.kind == ChannelType::GuildText
            && access
                .in_channel(&visitor, c, None)
                .contains(Permissions::VIEW_CHANNEL)
    }))
}

pub async fn resolve(ports: &Ports, code: &str) -> AppResult<(Invite, bool)> {
    if let Some(invite) = ports.invites.find(code).await? {
        return Ok((invite, false));
    }
    let vanity = ports
        .invites
        .vanity_by_code(code)
        .await?
        .ok_or(AppError::NotFound(Resource::Invite))?;
    let channel = public_channel(ports, vanity.guild_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Invite))?;
    Ok((
        Invite {
            code: vanity.code,
            guild_id: vanity.guild_id,
            channel_id: channel.id,
            inviter_id: None,
            max_age: 0,
            max_uses: 0,
            uses: vanity.uses,
            temporary: false,
            created_at: vanity.created_at,
        },
        true,
    ))
}
