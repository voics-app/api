use crate::{
    context::{Actor, Ports},
    error::{AppError, AppResult, Resource},
    ports::{PublicWidget, WidgetSettings},
    support::guild_context::GuildContext,
    use_cases::invites::get_invite::online_count,
};
use domain::{
    access::GuildAccess, channel::ChannelType, guild::Member, permissions::Permissions, Snowflake,
};

/// A channel the @everyone role can view.
#[derive(Debug, Clone)]
pub struct WidgetChannel {
    pub id: Snowflake,
    pub name: String,
    pub position: i32,
}

/// What `widget.json` shows anonymous visitors.
#[derive(Debug, Clone)]
pub struct WidgetView {
    pub id: Snowflake,
    pub name: String,
    pub icon: Option<String>,
    /// Code of a usable, non-temporary invite to the widget channel.
    pub instant_invite: Option<String>,
    pub presence_count: Option<u64>,
    pub channels: Vec<WidgetChannel>,
}

pub async fn public(ports: &Ports, guild_id: Snowflake) -> AppResult<WidgetView> {
    let PublicWidget { id, name, icon } = ports
        .guilds
        .public_widget(guild_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Guild))?;
    let settings = ports.guilds.widget_settings(guild_id).await?;
    let guild = ports
        .guilds
        .find(guild_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Guild))?;
    let roles = ports.guilds.roles(guild_id).await?;
    let now = ports.clock.now();
    let access = GuildAccess::new(&guild, &roles, now);
    // An anonymous visitor holds only the @everyone role.
    let visitor = Member {
        guild_id,
        user_id: Snowflake::new(0),
        nick: None,
        avatar: None,
        banner: None,
        bio: None,
        role_ids: Vec::new(),
        joined_at: now,
        deaf: false,
        mute: false,
        communication_disabled_until: None,
        pending: false,
    };
    let mut channels: Vec<WidgetChannel> = ports
        .channels
        .list_for_guild(guild_id)
        .await?
        .into_iter()
        .filter(|channel| {
            matches!(
                channel.kind,
                ChannelType::GuildText
                    | ChannelType::GuildVoice
                    | ChannelType::GuildAnnouncement
                    | ChannelType::GuildStageVoice
            ) && !channel.nsfw
                && access
                    .in_channel(&visitor, channel, None)
                    .contains(Permissions::VIEW_CHANNEL)
        })
        .filter_map(|channel| {
            Some(WidgetChannel {
                id: channel.id,
                name: channel.name?,
                position: channel.position,
            })
        })
        .collect();
    channels.sort_by_key(|channel| (channel.position, channel.id));
    let instant_invite = match settings.and_then(|s| s.channel_id) {
        Some(channel_id) if channels.iter().any(|c| c.id == channel_id) => ports
            .invites
            .list_for_guild(guild_id)
            .await?
            .into_iter()
            .filter(|invite| {
                invite.channel_id == channel_id && !invite.temporary && invite.is_usable(now)
            })
            .max_by_key(|invite| invite.created_at)
            .map(|invite| invite.code),
        _ => None,
    };
    Ok(WidgetView {
        id,
        name,
        icon,
        instant_invite,
        presence_count: online_count(ports, guild_id).await,
        channels,
    })
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
