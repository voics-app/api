//! `GET /internal/v1/channels/{id}/push-targets`: the members the notifier may push a new message
//! to, with the facts its rules need. Only members who can view the channel, who are not the
//! author and who hold a push token on a live session are listed.

use std::collections::HashMap;

use domain::access::GuildAccess;
use domain::channel::Channel;
use domain::delivery_mode::DeliveryMode;
use domain::notification::{GuildNotificationSettings, NotifyLevel, DIRECT_MESSAGES};
use domain::permissions::Permissions;
use domain::push_token::PushToken;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::{AppError, AppResult, Resource};
use crate::support::delivery::effective_mode;

/// Most targets one call returns, so a channel with a huge audience cannot produce an unbounded
/// answer. Members beyond it are not notified.
pub const MAX_TARGETS: usize = 5000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushTarget {
    pub user_id: Snowflake,
    pub role_ids: Vec<Snowflake>,
    /// The user holds a live gateway session.
    pub online: bool,
    /// The level that applies to this channel: its override, its category's, the guild's, the default.
    pub level: NotifyLevel,
    /// Muted now by the guild, the channel or its category.
    pub muted: bool,
    pub suppress_everyone: bool,
    pub suppress_roles: bool,
    pub tokens: Vec<PushToken>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushTargets {
    pub guild_id: Option<Snowflake>,
    pub guild_name: Option<String>,
    pub channel_name: Option<String>,
    pub delivery_mode: DeliveryMode,
    pub targets: Vec<PushTarget>,
}

struct Audience {
    /// The guild's name and its `default_message_notifications`.
    guild: Option<(String, u8)>,
    /// Users who may be notified, with the roles they hold in the guild.
    users: Vec<(Snowflake, Vec<Snowflake>)>,
}

async fn audience(ports: &Ports, channel: &Channel) -> AppResult<Audience> {
    let Some(guild_id) = channel.guild_id else {
        return Ok(Audience {
            guild: None,
            users: channel
                .recipient_ids
                .iter()
                .map(|id| (*id, Vec::new()))
                .collect(),
        });
    };
    let guild = ports
        .guilds
        .find(guild_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Guild))?;
    let roles = ports.guilds.roles(guild_id).await?;
    let parent = match (channel.kind.is_thread(), channel.parent_id) {
        (true, Some(parent_id)) => ports.channels.find(parent_id).await?,
        _ => None,
    };
    let access = GuildAccess::new(&guild, &roles, ports.clock.now());
    let users = ports
        .guilds
        .members(guild_id)
        .await?
        .into_iter()
        .filter(|member| {
            access
                .in_channel(member, channel, parent.as_ref())
                .contains(Permissions::VIEW_CHANNEL)
        })
        .map(|member| (member.user_id, member.role_ids))
        .collect();
    Ok(Audience {
        guild: Some((guild.name.clone(), guild.default_message_notifications)),
        users,
    })
}

pub async fn execute(
    ports: &Ports,
    channel_id: Snowflake,
    author_id: Snowflake,
) -> AppResult<PushTargets> {
    let channel = ports
        .channels
        .find(channel_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Channel))?;
    let delivery_mode = effective_mode(ports, &channel).await?;
    let Audience { guild, users } = audience(ports, &channel).await?;
    let candidates: Vec<Snowflake> = users
        .iter()
        .map(|(id, _)| *id)
        .filter(|id| *id != author_id)
        .collect();
    let mut tokens: HashMap<Snowflake, Vec<PushToken>> = HashMap::new();
    for token in ports.push_tokens.list_live_for_users(&candidates).await? {
        tokens.entry(token.user_id).or_default().push(token);
    }
    let mut roles: HashMap<Snowflake, Vec<Snowflake>> = users.into_iter().collect();
    let mut with_tokens: Vec<Snowflake> = tokens.keys().copied().collect();
    with_tokens.sort();
    with_tokens.truncate(MAX_TARGETS);
    let live = ports.presence.live_ids(&with_tokens).await?;
    let settings_scope = channel.guild_id.unwrap_or(DIRECT_MESSAGES);
    let settings: HashMap<Snowflake, GuildNotificationSettings> = ports
        .users
        .guild_notification_settings_many(&with_tokens, settings_scope)
        .await?
        .into_iter()
        .map(|settings| (settings.user_id, settings))
        .collect();
    let guild_default = match guild.as_ref().map(|(_, level)| *level) {
        Some(1) => NotifyLevel::OnlyMentions,
        _ => NotifyLevel::AllMessages,
    };
    let now = ports.clock.now();
    let targets = with_tokens
        .into_iter()
        .map(|user_id| {
            let own = settings.get(&user_id).cloned().unwrap_or_else(|| {
                GuildNotificationSettings::defaults_for(user_id, settings_scope)
            });
            PushTarget {
                user_id,
                role_ids: roles.remove(&user_id).unwrap_or_default(),
                online: live.contains(&user_id),
                level: own.effective_level_in(channel.id, channel.parent_id, guild_default),
                muted: own.is_muted_in(channel.id, channel.parent_id, now),
                suppress_everyone: own.suppress_everyone,
                suppress_roles: own.suppress_roles,
                tokens: tokens.remove(&user_id).unwrap_or_default(),
            }
        })
        .collect();
    Ok(PushTargets {
        guild_id: channel.guild_id,
        guild_name: guild.map(|(name, _)| name),
        channel_name: channel.name.clone(),
        delivery_mode,
        targets,
    })
}
