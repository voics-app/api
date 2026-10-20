use domain::notification::{GuildNotificationPatch, GuildNotificationSettings, DIRECT_MESSAGES};
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::events::DomainEvent;
use crate::write_set::{Mutation, WriteSet};

use super::get_guild_notification_settings::scope;

/// An override may only name a channel of the guild (or a DM the caller takes part in).
async fn check_channel(
    ports: &Ports,
    actor: &Actor,
    key: Snowflake,
    channel_id: Snowflake,
) -> AppResult<()> {
    let channel = ports
        .channels
        .find(channel_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Channel))?;
    let belongs = if key == DIRECT_MESSAGES {
        channel.guild_id.is_none() && channel.is_recipient(actor.user_id)
    } else {
        channel.guild_id == Some(key)
    };
    if belongs {
        Ok(())
    } else {
        Err(AppError::invalid(
            "channel_overrides",
            "NOTIFICATION_CHANNEL_SCOPE",
            "Channel does not belong to this server.",
        ))
    }
}

/// `PATCH /users/@me/guilds/{id}/settings` (`guild_id: None` for DMs).
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Option<Snowflake>,
    patch: GuildNotificationPatch,
) -> AppResult<GuildNotificationSettings> {
    actor.require_user()?;
    let key = scope(ports, actor, guild_id).await?;
    for entry in patch.channel_overrides.iter().flatten() {
        let clearing =
            entry.level == Some(None) && entry.muted == Some(false) && entry.mute_config.is_none();
        // Clearing an override must keep working after its channel was deleted.
        if !clearing {
            check_channel(ports, actor, key, entry.channel_id).await?;
        }
    }
    let mut settings = ports
        .users
        .guild_notification_settings(actor.user_id, key)
        .await?
        .unwrap_or_else(|| GuildNotificationSettings::defaults_for(actor.user_id, key));
    settings.apply(patch, ports.clock.now())?;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpsertGuildNotificationSettings(settings.clone()))
        .emit(DomainEvent::UserGuildSettingsUpdate(settings.clone()));
    ports.unit_of_work.commit(writes).await?;
    Ok(settings)
}
