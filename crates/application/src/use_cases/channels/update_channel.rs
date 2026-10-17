use serde_json::json;

use domain::audit::{AuditAction, AuditChange};
use domain::channel::{Channel, ChannelType, AUTO_ARCHIVE_DURATIONS};
use domain::delivery_mode::DeliveryMode;
use domain::permissions::{PermissionOverwrite, Permissions};
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::{ChannelView, DomainEvent};
use crate::support::audit::AuditContext;
use crate::support::channel_access::ChannelAccess;
use crate::support::guild_context::GuildContext;
use crate::support::images::{store_hashed, ImageUpload};
use crate::support::views::private_view;
use crate::use_cases::channels::channel_rules::*;
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone, Default)]
pub struct UpdateChannelInput {
    pub name: Option<String>,
    pub topic: Option<Option<String>>,
    pub icon: Option<Option<ImageUpload>>,
    pub bitrate: Option<u32>,
    pub user_limit: Option<u32>,
    pub rate_limit_per_user: Option<u32>,
    pub position: Option<i32>,
    pub parent_id: Option<Option<Snowflake>>,
    pub nsfw: Option<bool>,
    pub permission_overwrites: Option<Vec<PermissionOverwrite>>,
    pub delivery_mode: Option<DeliveryMode>,
    pub voice_region: Option<Option<String>>,
    pub archived: Option<bool>,
    pub locked: Option<bool>,
    pub auto_archive_duration: Option<u32>,
}

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    channel_id: Snowflake,
    input: UpdateChannelInput,
    audit: AuditContext<'_>,
) -> AppResult<ChannelView> {
    let access = ChannelAccess::visible(ports, channel_id, actor.user_id).await?;
    match access.guild {
        None => update_group_dm(ports, access.channel, input).await,
        Some(ref ctx) if access.channel.kind.is_thread() => update_thread(ports, actor, ctx, &access, input, audit).await,
        Some(ref ctx) => update_guild_channel(ports, ctx, &access, input, audit).await,
    }
}

async fn update_group_dm(ports: &Ports, mut channel: Channel, input: UpdateChannelInput) -> AppResult<ChannelView> {
    if channel.kind != ChannelType::GroupDm {
        return Err(AppError::missing_permissions());
    }
    if let Some(name) = input.name {
        apply_name(&mut channel, &name)?;
    }
    if let Some(icon) = input.icon {
        channel.icon = match icon {
            Some(image) => Some(store_hashed(ports, &format!("channel-icons/{}", channel.id), image).await?),
            None => None,
        };
    }
    let view = private_view(ports, channel.clone()).await?;
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::UpdateChannel(channel)).emit(DomainEvent::PrivateChannelUpdate(view.clone()));
    ports.unit_of_work.commit(writes).await?;
    Ok(ChannelView::Private(view))
}

async fn update_thread(
    ports: &Ports,
    actor: &Actor,
    ctx: &GuildContext,
    access: &ChannelAccess,
    input: UpdateChannelInput,
    audit: AuditContext<'_>,
) -> AppResult<ChannelView> {
    let manages = access.permissions.contains(Permissions::MANAGE_THREADS);
    if !manages && access.channel.owner_id != Some(actor.user_id) {
        return Err(AppError::missing_permissions());
    }
    let mut thread = access.channel.clone();
    if let Some(name) = input.name {
        apply_name(&mut thread, &name)?;
    }
    if let Some(seconds) = input.rate_limit_per_user {
        apply_slowmode(&mut thread, seconds)?;
    }
    if let Some(metadata) = thread.thread_metadata.as_mut() {
        if let Some(archived) = input.archived {
            if metadata.locked && !manages {
                return Err(AppError::missing_permissions());
            }
            metadata.archived = archived;
            metadata.archive_timestamp = ports.clock.now();
        }
        if let Some(locked) = input.locked {
            if !manages {
                return Err(AppError::missing_permissions());
            }
            metadata.locked = locked;
        }
        if let Some(duration) = input.auto_archive_duration {
            if !AUTO_ARCHIVE_DURATIONS.contains(&duration) {
                return Err(AppError::invalid("auto_archive_duration", "BASE_TYPE_CHOICES", "Invalid duration."));
            }
            metadata.auto_archive_duration = duration;
        }
    }
    let changes = name_change(&access.channel, &thread);
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpdateChannel(thread.clone()))
        .emit(DomainEvent::ThreadUpdate(thread.clone()))
        .audit(audit.entry(ports, ctx.guild.id, AuditAction::ThreadUpdate, Some(thread.id), changes)?);
    ports.unit_of_work.commit(writes).await?;
    Ok(ChannelView::Guild(thread))
}

async fn update_guild_channel(
    ports: &Ports,
    ctx: &GuildContext,
    access: &ChannelAccess,
    input: UpdateChannelInput,
    audit: AuditContext<'_>,
) -> AppResult<ChannelView> {
    access.require(Permissions::MANAGE_CHANNELS)?;
    let before = &access.channel;
    let mut channel = before.clone();
    if let Some(name) = input.name {
        apply_name(&mut channel, &name)?;
    }
    if let Some(topic) = input.topic {
        apply_topic(&mut channel, topic)?;
    }
    if let Some(bitrate) = input.bitrate {
        apply_bitrate(&mut channel, bitrate)?;
    }
    if let Some(limit) = input.user_limit {
        apply_user_limit(&mut channel, limit)?;
    }
    if let Some(seconds) = input.rate_limit_per_user {
        apply_slowmode(&mut channel, seconds)?;
    }
    if let Some(region) = input.voice_region {
        apply_voice_region(ports, &mut channel, region)?;
    }
    if let Some(parent_id) = input.parent_id {
        apply_parent(ports, &mut channel, parent_id).await?;
    }
    if let Some(position) = input.position {
        channel.position = position;
    }
    if let Some(nsfw) = input.nsfw {
        channel.nsfw = nsfw;
    }
    if let Some(overwrites) = input.permission_overwrites {
        check_overwrites(ctx, access.permissions, &overwrites)?;
        channel.overwrites = overwrites;
    }
    let mode = input.delivery_mode.unwrap_or(channel.delivery_mode);
    if input.delivery_mode.is_some() || (mode == DeliveryMode::Ephemeral && channel.overwrites != before.overwrites) {
        check_delivery_mode(ports, ctx, &channel, mode).await?;
    }
    channel.delivery_mode = mode;

    let mut changes = name_change(before, &channel);
    changes.extend(
        [
            AuditChange::if_changed("topic", json!(before.topic), json!(channel.topic)),
            AuditChange::if_changed("delivery_mode", json!(before.delivery_mode.as_str()), json!(channel.delivery_mode.as_str())),
            AuditChange::if_changed("nsfw", json!(before.nsfw), json!(channel.nsfw)),
            AuditChange::if_changed("rate_limit_per_user", json!(before.rate_limit_per_user), json!(channel.rate_limit_per_user)),
            AuditChange::if_changed("bitrate", json!(before.bitrate), json!(channel.bitrate)),
        ]
        .into_iter()
        .flatten(),
    );
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::UpdateChannel(channel.clone())).emit(DomainEvent::ChannelUpdate(channel.clone()));
    if !changes.is_empty() {
        writes.audit(audit.entry(ports, ctx.guild.id, AuditAction::ChannelUpdate, Some(channel.id), changes)?);
    }
    ports.unit_of_work.commit(writes).await?;
    Ok(ChannelView::Guild(channel))
}

fn name_change(before: &Channel, after: &Channel) -> Vec<AuditChange> {
    AuditChange::if_changed("name", json!(before.name), json!(after.name)).into_iter().collect()
}
