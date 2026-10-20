use domain::notification::{GuildNotificationSettings, DIRECT_MESSAGES};
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};

/// Confirms the caller belongs to `guild_id` (`None` = DMs, always allowed) and returns the
/// stored settings key.
pub(super) async fn scope(
    ports: &Ports,
    actor: &Actor,
    guild_id: Option<Snowflake>,
) -> AppResult<Snowflake> {
    let Some(guild_id) = guild_id else {
        return Ok(DIRECT_MESSAGES);
    };
    if ports.guilds.find(guild_id).await?.is_none() {
        return Err(AppError::NotFound(Resource::Guild));
    }
    if ports
        .guilds
        .member(guild_id, actor.user_id)
        .await?
        .is_none()
    {
        return Err(AppError::missing_access());
    }
    Ok(guild_id)
}

/// Stored settings of one guild or of DMs (`guild_id: None`), or the defaults.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Option<Snowflake>,
) -> AppResult<GuildNotificationSettings> {
    actor.require_user()?;
    let key = scope(ports, actor, guild_id).await?;
    Ok(ports
        .users
        .guild_notification_settings(actor.user_id, key)
        .await?
        .unwrap_or_else(|| GuildNotificationSettings::defaults_for(actor.user_id, key)))
}
