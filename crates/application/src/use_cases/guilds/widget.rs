use crate::{
    context::{Actor, Ports},
    error::{AppError, AppResult, Resource},
    ports::{PublicWidget, WidgetSettings},
    support::guild_context::GuildContext,
};
use domain::{permissions::Permissions, Snowflake};

pub async fn public(ports: &Ports, guild_id: Snowflake) -> AppResult<PublicWidget> {
    ports
        .guilds
        .public_widget(guild_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Guild))
}

pub async fn update(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    enabled: Option<bool>,
    channel: Option<Option<Snowflake>>,
) -> AppResult<WidgetSettings> {
    GuildContext::load(ports, guild_id, actor.user_id)
        .await?
        .require(Permissions::MANAGE_GUILD)?;
    ports
        .guilds
        .update_widget_settings(guild_id, enabled, channel)
        .await
}

pub async fn settings(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
) -> AppResult<WidgetSettings> {
    GuildContext::load(ports, guild_id, actor.user_id)
        .await?
        .require(Permissions::MANAGE_GUILD)?;
    ports
        .guilds
        .widget_settings(guild_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Guild))
}
