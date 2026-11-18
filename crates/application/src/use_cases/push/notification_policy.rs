//! Shared current-policy resolution for push targets and token-independent mention recipients.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use domain::channel::Channel;
use domain::notification::{GuildNotificationSettings, NotifyLevel, DIRECT_MESSAGES};
use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationRecipient {
    pub user_id: Snowflake,
    pub role_ids: Vec<Snowflake>,
    pub online: bool,
    pub level: NotifyLevel,
    pub muted: bool,
    pub suppress_everyone: bool,
    pub suppress_roles: bool,
}

pub(super) async fn resolve(
    ports: &Ports,
    channel: &Channel,
    guild_default: NotifyLevel,
    now: DateTime<Utc>,
    users: Vec<(Snowflake, Vec<Snowflake>)>,
) -> AppResult<Vec<NotificationRecipient>> {
    if users.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<_> = users.iter().map(|(id, _)| *id).collect();
    let live: HashSet<_> = ports.presence.live_ids(&ids).await?.into_iter().collect();
    let scope = channel.guild_id.unwrap_or(DIRECT_MESSAGES);
    let settings: HashMap<_, _> = ports
        .users
        .guild_notification_settings_many(&ids, scope)
        .await?
        .into_iter()
        .map(|settings| (settings.user_id, settings))
        .collect();
    Ok(users
        .into_iter()
        .map(|(user_id, mut role_ids)| {
            role_ids.sort();
            role_ids.dedup();
            let own = settings
                .get(&user_id)
                .cloned()
                .unwrap_or_else(|| GuildNotificationSettings::defaults_for(user_id, scope));
            NotificationRecipient {
                user_id,
                role_ids,
                online: live.contains(&user_id),
                level: own.effective_level_in(channel.id, channel.parent_id, guild_default),
                muted: own.is_muted_in(channel.id, channel.parent_id, now),
                suppress_everyone: own.suppress_everyone,
                suppress_roles: own.suppress_roles,
            }
        })
        .collect())
}

pub(super) fn guild_default(code: u8) -> NotifyLevel {
    if code == 1 {
        NotifyLevel::OnlyMentions
    } else {
        NotifyLevel::AllMessages
    }
}
