use domain::channel::ChannelType;
use domain::permissions::Permissions;
use domain::Snowflake;
use serde_json::Value;

use crate::context::{Actor, Ports};
use crate::support::guild_context::GuildContext;
use crate::{AppError, AppResult};

pub async fn get(ports: &Ports, actor: &Actor, guild_id: Snowflake) -> AppResult<Value> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    let mut value = ports.guilds.welcome_screen(guild_id).await?;
    if let Some(channels) = value.get_mut("channels").and_then(Value::as_array_mut) {
        let mut visible = Vec::new();
        for entry in channels.drain(..) {
            let Some(id) = entry["channel_id"]
                .as_str()
                .and_then(|s| s.parse::<Snowflake>().ok())
            else {
                continue;
            };
            let Some(channel) = ports.channels.find(id).await? else {
                continue;
            };
            if channel.guild_id == Some(guild_id)
                && ctx
                    .channel_permissions(&channel, None)
                    .contains(Permissions::VIEW_CHANNEL)
            {
                visible.push(entry);
            }
        }
        *channels = visible;
    }
    Ok(value)
}

pub async fn set(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    value: Value,
) -> AppResult<Value> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_GUILD)?;
    let invalid = || {
        AppError::invalid(
            "welcome_screen",
            "BASE_TYPE_INVALID",
            "Provide a description and up to five visible text channels with descriptions.",
        )
    };
    let object = value.as_object().ok_or_else(invalid)?;
    if object.len() != 3
        || !object.get("enabled").is_some_and(Value::is_boolean)
        || !object
            .get("description")
            .and_then(Value::as_str)
            .is_some_and(|s| s.chars().count() <= 300)
    {
        return Err(invalid());
    }
    let channels = object
        .get("channels")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    if channels.len() > 5 {
        return Err(invalid());
    }
    let mut ids = std::collections::HashSet::new();
    for entry in channels {
        let item = entry.as_object().ok_or_else(invalid)?;
        let id = item
            .get("channel_id")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<Snowflake>().ok())
            .ok_or_else(invalid)?;
        if item.len() != 2
            || !ids.insert(id)
            || !item
                .get("description")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= 100)
        {
            return Err(invalid());
        }
        let channel = ports.channels.find(id).await?.ok_or_else(invalid)?;
        if channel.guild_id != Some(guild_id)
            || !matches!(
                channel.kind,
                ChannelType::GuildText | ChannelType::GuildAnnouncement
            )
            || !ctx
                .channel_permissions(&channel, None)
                .contains(Permissions::VIEW_CHANNEL)
        {
            return Err(invalid());
        }
    }
    ports.guilds.set_welcome_screen(guild_id, &value).await?;
    Ok(value)
}
