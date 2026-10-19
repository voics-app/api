use serde_json::json;

use domain::audit::{AuditAction, AuditChange};
use domain::channel::{Channel, ChannelType};
use domain::delivery_mode::DeliveryMode;
use domain::permissions::{PermissionOverwrite, Permissions};
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::guild_context::GuildContext;
use crate::use_cases::channels::channel_rules::*;
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone, Default)]
pub struct CreateChannelInput {
    pub name: String,
    pub kind: Option<u8>,
    pub topic: Option<String>,
    pub bitrate: Option<u32>,
    pub user_limit: Option<u32>,
    pub rate_limit_per_user: Option<u32>,
    pub position: Option<i32>,
    pub parent_id: Option<Snowflake>,
    pub nsfw: Option<bool>,
    pub permission_overwrites: Vec<PermissionOverwrite>,
    pub delivery_mode: Option<DeliveryMode>,
    pub voice_region: Option<String>,
}

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    input: CreateChannelInput,
    audit: AuditContext<'_>,
) -> AppResult<Channel> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_CHANNELS)?;
    let kind = ChannelType::from_code(input.kind.unwrap_or(0))
        .filter(|k| k.is_creatable_in_guild())
        .ok_or_else(|| AppError::invalid("type", "BASE_TYPE_CHOICES", "Invalid channel type."))?;
    let existing = ports.channels.list_for_guild(guild_id).await?;
    if existing.len() >= MAX_CHANNELS_PER_GUILD {
        return Err(AppError::BadRequest {
            code: 30013,
            message: "Maximum number of guild channels reached (500)".into(),
        });
    }

    let mut channel = Channel::new(ports.ids.next_id(), kind, Some(guild_id), ports.clock.now());
    apply_name(&mut channel, &input.name)?;
    apply_topic(&mut channel, input.topic)?;
    if let Some(bitrate) = input.bitrate {
        apply_bitrate(&mut channel, bitrate)?;
    }
    if let Some(limit) = input.user_limit {
        apply_user_limit(&mut channel, limit)?;
    }
    if let Some(seconds) = input.rate_limit_per_user {
        apply_slowmode(&mut channel, seconds)?;
    }
    apply_voice_region(ports, &mut channel, input.voice_region)?;
    apply_parent(ports, &mut channel, input.parent_id).await?;
    channel.nsfw = input.nsfw.unwrap_or(false);
    channel.position = input.position.unwrap_or_else(|| {
        existing
            .iter()
            .filter(|c| c.parent_id == channel.parent_id)
            .map(|c| c.position + 1)
            .max()
            .unwrap_or(0)
    });
    if !input.permission_overwrites.is_empty() {
        check_overwrites(&ctx, ctx.permissions(), &input.permission_overwrites)?;
        channel.overwrites = input.permission_overwrites;
    } else if let Some(parent) = existing.iter().find(|c| Some(c.id) == channel.parent_id) {
        channel.overwrites = parent.overwrites.clone();
    }
    let mode = input.delivery_mode.unwrap_or_default();
    check_delivery_mode(ports, &ctx, &channel, mode).await?;
    channel.delivery_mode = mode;

    let changes = vec![
        AuditChange::new("name", None, Some(json!(channel.name))),
        AuditChange::new("type", None, Some(json!(channel.kind.code()))),
    ];
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::InsertChannel(channel.clone()))
        .emit(DomainEvent::ChannelCreate(channel.clone()))
        .audit(audit.entry(
            ports,
            guild_id,
            AuditAction::ChannelCreate,
            Some(channel.id),
            changes,
        )?);
    ports.unit_of_work.commit(writes).await?;
    Ok(channel)
}
