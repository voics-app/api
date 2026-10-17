//! Field rules shared by channel creation and update.

use domain::channel::{Channel, ChannelType};
use domain::delivery_mode::{validate_guild_channel_mode, DeliveryMode};
use domain::permissions::{OverwriteKind, PermissionOverwrite, Permissions};
use domain::validation;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::{AppError, AppResult};
use crate::support::delivery::viewer_count;
use crate::support::guild_context::GuildContext;
use crate::use_cases::guilds::create_guild::ensure_known_region;

pub const MAX_CHANNELS_PER_GUILD: usize = 500;
pub const MAX_OVERWRITES: usize = 100;

pub fn apply_name(channel: &mut Channel, name: &str) -> AppResult<()> {
    validation::channel_name(name)?;
    let name = if channel.kind.has_text_style_name() {
        validation::normalize_text_channel_name(name)
    } else {
        name.trim().to_owned()
    };
    channel.name = Some(name);
    Ok(())
}

pub fn apply_topic(channel: &mut Channel, topic: Option<String>) -> AppResult<()> {
    if let Some(topic) = &topic {
        validation::channel_topic(topic)?;
    }
    channel.topic = topic;
    Ok(())
}

pub fn apply_bitrate(channel: &mut Channel, bitrate: u32) -> AppResult<()> {
    if !channel.kind.is_voice() || !(8_000..=384_000).contains(&bitrate) {
        return Err(AppError::invalid("bitrate", "BASE_TYPE_BAD_INT", "Bitrate must be between 8000 and 384000."));
    }
    channel.bitrate = Some(bitrate);
    Ok(())
}

pub fn apply_user_limit(channel: &mut Channel, limit: u32) -> AppResult<()> {
    if !channel.kind.is_voice() || limit > 99 {
        return Err(AppError::invalid("user_limit", "BASE_TYPE_BAD_INT", "User limit must be between 0 and 99."));
    }
    channel.user_limit = Some(limit);
    Ok(())
}

pub fn apply_slowmode(channel: &mut Channel, seconds: u32) -> AppResult<()> {
    if seconds > 21_600 {
        return Err(AppError::invalid("rate_limit_per_user", "BASE_TYPE_BAD_INT", "Must be between 0 and 21600."));
    }
    channel.rate_limit_per_user = seconds;
    Ok(())
}

pub fn apply_voice_region(ports: &Ports, channel: &mut Channel, region: Option<String>) -> AppResult<()> {
    if let Some(region) = &region {
        if !channel.kind.is_voice() {
            return Err(AppError::invalid("voice_region", "CHANNEL_TYPE_INVALID", "Only voice channels have regions."));
        }
        ensure_known_region(ports, region)?;
    }
    channel.voice_region = region;
    Ok(())
}

/// Parent must be a category of the same guild (and not the channel itself).
pub async fn apply_parent(
    ports: &Ports,
    channel: &mut Channel,
    parent_id: Option<Snowflake>,
) -> AppResult<()> {
    if let Some(parent_id) = parent_id {
        let valid = channel.kind != ChannelType::GuildCategory
            && parent_id != channel.id
            && ports
                .channels
                .find(parent_id)
                .await?
                .is_some_and(|p| p.guild_id == channel.guild_id && p.kind == ChannelType::GuildCategory);
        if !valid {
            return Err(AppError::invalid("parent_id", "CHANNEL_PARENT_INVALID", "Parent must be a category."));
        }
    }
    channel.parent_id = parent_id;
    Ok(())
}

/// The caller needs MANAGE_ROLES and may only allow or deny what they hold
/// (`held`: guild permissions on create, channel permissions on update).
pub fn check_overwrites(ctx: &GuildContext, held: Permissions, overwrites: &[PermissionOverwrite]) -> AppResult<()> {
    if overwrites.len() > MAX_OVERWRITES {
        return Err(AppError::invalid("permission_overwrites", "BASE_TYPE_MAX_LENGTH", "Too many overwrites."));
    }
    if !held.contains(Permissions::MANAGE_ROLES) {
        return Err(AppError::missing_permissions());
    }
    let requested = overwrites.iter().fold(Permissions::NONE, |acc, o| acc | o.allow | o.deny);
    if !held.contains(requested) {
        return Err(AppError::missing_permissions());
    }
    for overwrite in overwrites {
        if overwrite.kind == OverwriteKind::Role && !ctx.roles.iter().any(|r| r.id == overwrite.id) {
            return Err(AppError::invalid("permission_overwrites", "ROLE_INVALID", "Unknown role in overwrite."));
        }
    }
    Ok(())
}

/// Validates the configured delivery mode against P11 constraints.
pub async fn check_delivery_mode(ports: &Ports, ctx: &GuildContext, channel: &Channel, mode: DeliveryMode) -> AppResult<()> {
    if channel.kind.is_thread() || channel.kind == ChannelType::GuildCategory {
        if mode != DeliveryMode::Standard {
            return Err(AppError::invalid("delivery_mode", "DELIVERY_MODE_INVALID", "Not supported for this channel type."));
        }
        return Ok(());
    }
    let viewers = match mode {
        DeliveryMode::Ephemeral => viewer_count(ports, &ctx.guild, &ctx.roles, channel, None).await?,
        _ => 0,
    };
    validate_guild_channel_mode(mode, viewers)?;
    Ok(())
}
