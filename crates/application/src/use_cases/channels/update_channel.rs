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
use crate::support::system_messages;
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
    pub text_chat_enabled: Option<bool>,
    pub permission_overwrites: Option<Vec<PermissionOverwrite>>,
    pub delivery_mode: Option<DeliveryMode>,
    pub voice_region: Option<Option<String>>,
    pub archived: Option<bool>,
    pub locked: Option<bool>,
    pub auto_archive_duration: Option<u32>,
    pub available_tags: Option<serde_json::Value>,
    pub default_sort_order: Option<Option<u8>>,
    pub default_forum_layout: Option<u8>,
    pub default_reaction_emoji: Option<Option<serde_json::Value>>,
    pub require_tags: Option<bool>,
    pub applied_tags: Option<Vec<String>>,
    pub flags: Option<u32>,
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
        None => update_group_dm(ports, actor, access.channel, input).await,
        Some(ref ctx) if access.channel.kind.is_thread() => {
            update_thread(ports, actor, ctx, &access, input, audit).await
        }
        Some(ref ctx) => update_guild_channel(ports, ctx, &access, input, audit).await,
    }
}

async fn update_group_dm(
    ports: &Ports,
    actor: &Actor,
    mut channel: Channel,
    input: UpdateChannelInput,
) -> AppResult<ChannelView> {
    if channel.kind != ChannelType::GroupDm {
        return Err(AppError::missing_permissions());
    }
    let before = channel.clone();
    if let Some(name) = input.name {
        apply_name(&mut channel, &name)?;
    }
    if let Some(icon) = input.icon {
        channel.icon = match icon {
            Some(image) => {
                Some(store_hashed(ports, &format!("channel-icons/{}", channel.id), image).await?)
            }
            None => None,
        };
    }
    let view = private_view(ports, channel.clone()).await?;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpdateChannel(channel.clone()))
        .emit(DomainEvent::PrivateChannelUpdate(view.clone()));
    ports.unit_of_work.commit(writes).await?;
    if channel.name != before.name {
        system_messages::post(
            ports,
            view.delivery_mode,
            channel.id,
            actor.user_id,
            system_messages::CHANNEL_NAME_CHANGE,
            None,
            channel.name.as_deref(),
        )
        .await;
    }
    if channel.icon != before.icon {
        system_messages::post(
            ports,
            view.delivery_mode,
            channel.id,
            actor.user_id,
            system_messages::CHANNEL_ICON_CHANGE,
            None,
            None,
        )
        .await;
    }
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
    if let Some(flags) = input.flags {
        if !manages {
            return Err(AppError::missing_permissions());
        }
        let parent_id = thread.parent_id.ok_or_else(|| {
            AppError::invalid("flags", "CHANNEL_TYPE_INVALID", "Thread has no parent.")
        })?;
        let parent = ports.channels.find(parent_id).await?.ok_or_else(|| {
            AppError::invalid("flags", "CHANNEL_TYPE_INVALID", "Thread parent not found.")
        })?;
        if parent.kind != ChannelType::GuildForum || flags & !2 != 0 {
            return Err(AppError::invalid(
                "flags",
                "CHANNEL_TYPE_INVALID",
                "Only forum posts can be pinned.",
            ));
        }
        thread.forum_settings["pinned"] = json!(flags & 2 != 0);
    }
    if let Some(tags) = input.applied_tags.as_ref() {
        let parent_id = thread.parent_id.ok_or_else(|| {
            AppError::invalid(
                "applied_tags",
                "CHANNEL_TYPE_INVALID",
                "Thread has no parent.",
            )
        })?;
        let parent = ports.channels.find(parent_id).await?.ok_or_else(|| {
            AppError::invalid(
                "applied_tags",
                "CHANNEL_TYPE_INVALID",
                "Thread parent not found.",
            )
        })?;
        super::forum::validate_applied(&parent, tags, manages)?;
        thread.applied_tags = tags.clone();
    }
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
                return Err(AppError::invalid(
                    "auto_archive_duration",
                    "BASE_TYPE_CHOICES",
                    "Invalid duration.",
                ));
            }
            metadata.auto_archive_duration = duration;
        }
    }
    let mut changes = name_change(&access.channel, &thread);
    changes.extend(
        [
            AuditChange::if_changed(
                "applied_tags",
                json!(access.channel.applied_tags),
                json!(thread.applied_tags),
            ),
            AuditChange::if_changed(
                "pinned",
                json!(access.channel.forum_settings.get("pinned")),
                json!(thread.forum_settings.get("pinned")),
            ),
        ]
        .into_iter()
        .flatten(),
    );
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpdateChannel(thread.clone()))
        .emit(DomainEvent::ThreadUpdate(thread.clone()))
        .audit(audit.entry(
            ports,
            ctx.guild.id,
            AuditAction::ThreadUpdate,
            Some(thread.id),
            changes,
        )?);
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
    let has_forum_fields = input.available_tags.is_some()
        || input.default_sort_order.is_some()
        || input.default_forum_layout.is_some()
        || input.default_reaction_emoji.is_some()
        || input.require_tags.is_some();
    if has_forum_fields && channel.kind != ChannelType::GuildForum {
        return Err(AppError::invalid(
            "available_tags",
            "CHANNEL_TYPE_INVALID",
            "Forum settings require a forum channel.",
        ));
    }
    if input.flags.is_some() {
        return Err(AppError::invalid(
            "flags",
            "CHANNEL_TYPE_INVALID",
            "Only forum posts can be pinned.",
        ));
    }
    if input.applied_tags.is_some() {
        return Err(AppError::invalid(
            "applied_tags",
            "CHANNEL_TYPE_INVALID",
            "Tags apply to forum posts.",
        ));
    }
    if let Some(tags) = input.available_tags {
        channel.forum_settings["available_tags"] =
            super::forum::normalize_tags(ports, before, tags)?;
    }
    if let Some(sort) = input.default_sort_order {
        if sort.is_some_and(|v| v > 1) {
            return Err(AppError::invalid(
                "default_sort_order",
                "BASE_TYPE_CHOICES",
                "Invalid sort order.",
            ));
        }
        channel.forum_settings["default_sort_order"] = json!(sort);
    }
    if let Some(layout) = input.default_forum_layout {
        if layout > 2 {
            return Err(AppError::invalid(
                "default_forum_layout",
                "BASE_TYPE_CHOICES",
                "Invalid layout.",
            ));
        }
        channel.forum_settings["default_forum_layout"] = json!(layout);
    }
    if let Some(reaction) = input.default_reaction_emoji {
        if let Some(value) = reaction.as_ref() {
            super::forum::validate_emoji(value, "default_reaction_emoji")?;
        }
        channel.forum_settings["default_reaction_emoji"] = json!(reaction);
    }
    if let Some(required) = input.require_tags {
        channel.forum_settings["require_tags"] = json!(required);
    }
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
    if let Some(enabled) = input.text_chat_enabled {
        if !channel.kind.is_voice() {
            return Err(AppError::invalid(
                "text_chat_enabled",
                "CHANNEL_TYPE_INVALID",
                "Only voice channels have a text chat.",
            ));
        }
        channel.text_chat_enabled = enabled;
    }
    if let Some(overwrites) = input.permission_overwrites {
        check_overwrites(ctx, access.permissions, &overwrites)?;
        channel.overwrites = overwrites;
    }
    let mode = input.delivery_mode.unwrap_or(channel.delivery_mode);
    if input.delivery_mode.is_some()
        || (mode == DeliveryMode::Ephemeral && channel.overwrites != before.overwrites)
    {
        check_delivery_mode(ports, ctx, &channel, mode).await?;
    }
    channel.delivery_mode = mode;

    let mut changes = name_change(before, &channel);
    changes.extend(
        [
            AuditChange::if_changed("topic", json!(before.topic), json!(channel.topic)),
            AuditChange::if_changed(
                "forum_settings",
                json!(before.forum_settings),
                json!(channel.forum_settings),
            ),
            AuditChange::if_changed(
                "delivery_mode",
                json!(before.delivery_mode.as_str()),
                json!(channel.delivery_mode.as_str()),
            ),
            AuditChange::if_changed("nsfw", json!(before.nsfw), json!(channel.nsfw)),
            AuditChange::if_changed(
                "text_chat_enabled",
                json!(before.text_chat_enabled),
                json!(channel.text_chat_enabled),
            ),
            AuditChange::if_changed(
                "rate_limit_per_user",
                json!(before.rate_limit_per_user),
                json!(channel.rate_limit_per_user),
            ),
            AuditChange::if_changed("bitrate", json!(before.bitrate), json!(channel.bitrate)),
        ]
        .into_iter()
        .flatten(),
    );
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpdateChannel(channel.clone()))
        .emit(DomainEvent::ChannelUpdate(channel.clone()));
    if !changes.is_empty() {
        writes.audit(audit.entry(
            ports,
            ctx.guild.id,
            AuditAction::ChannelUpdate,
            Some(channel.id),
            changes,
        )?);
    }
    ports.unit_of_work.commit(writes).await?;
    Ok(ChannelView::Guild(channel))
}

fn name_change(before: &Channel, after: &Channel) -> Vec<AuditChange> {
    AuditChange::if_changed("name", json!(before.name), json!(after.name))
        .into_iter()
        .collect()
}
