//! Validating the Community channel pair for a partial guild update.
use domain::channel::ChannelType;
use domain::delivery_mode::DeliveryMode;
use domain::guild::{CommunityChannels, Guild};
use domain::Snowflake;

use super::update_guild::UpdateGuildInput;
use crate::{AppError, AppResult, Ports};

pub(super) async fn apply(
    ports: &Ports,
    guild: &mut Guild,
    input: &UpdateGuildInput,
) -> AppResult<bool> {
    let requested = input.features.is_some()
        || input.rules_channel_id.is_some()
        || input.public_updates_channel_id.is_some();
    if !requested {
        return Ok(false);
    }
    let enabled = match &input.features {
        None => guild.community.is_some(),
        Some(features) if features.is_empty() => false,
        Some(features) if features.as_slice() == ["COMMUNITY"] => true,
        Some(_) => {
            return Err(AppError::invalid(
                "features",
                "BASE_TYPE_CHOICES",
                "Supported features are [] or [COMMUNITY].",
            ))
        }
    };
    if !enabled {
        if input.rules_channel_id.flatten().is_some()
            || input.public_updates_channel_id.flatten().is_some()
        {
            return Err(AppError::invalid(
                "features",
                "COMMUNITY_REQUIRED",
                "Enable Community before configuring its channels.",
            ));
        }
        guild.community = None;
        return Ok(true);
    }
    let rules = input
        .rules_channel_id
        .unwrap_or(guild.community.map(|c| c.rules_channel_id));
    let updates = input
        .public_updates_channel_id
        .unwrap_or(guild.community.map(|c| c.public_updates_channel_id));
    let rules = validate(ports, guild.id, rules, "rules_channel_id").await?;
    let updates = validate(ports, guild.id, updates, "public_updates_channel_id").await?;
    guild.community = Some(CommunityChannels {
        rules_channel_id: rules,
        public_updates_channel_id: updates,
    });
    Ok(true)
}

async fn validate(
    ports: &Ports,
    guild_id: Snowflake,
    id: Option<Snowflake>,
    field: &str,
) -> AppResult<Snowflake> {
    let invalid = || {
        AppError::invalid(
            field,
            "CHANNEL_INVALID",
            "Choose a non-age-restricted Standard text or announcement channel in this server.",
        )
    };
    let id = id.ok_or_else(invalid)?;
    let channel = ports.channels.find(id).await?.ok_or_else(invalid)?;
    if channel.guild_id != Some(guild_id)
        || !matches!(
            channel.kind,
            ChannelType::GuildText | ChannelType::GuildAnnouncement
        )
        || channel.nsfw
        || channel.delivery_mode != DeliveryMode::Standard
    {
        return Err(invalid());
    }
    Ok(id)
}
