//! Bounded recipient discovery for mention accounting, independent of mobile devices.

use chrono::{DateTime, Utc};
use domain::access::GuildAccess;
use domain::delivery_mode::DeliveryMode;
use domain::notification::NotifyLevel;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::{AppError, AppResult, Resource};
use crate::support::delivery::effective_mode;

use super::notification_policy::{self, NotificationRecipient};

pub const DEFAULT_LIMIT: u32 = 100;
pub const MAX_LIMIT: u32 = 200;

pub struct NotificationRecipientPage {
    pub channel_id: Snowflake,
    pub guild_id: Option<Snowflake>,
    pub delivery_mode: DeliveryMode,
    pub evaluated_at: DateTime<Utc>,
    pub recipients: Vec<NotificationRecipient>,
    pub next_after: Option<Snowflake>,
}

pub async fn execute(
    ports: &Ports,
    channel_id: Snowflake,
    author_id: Snowflake,
    after: Option<Snowflake>,
    limit: u32,
) -> AppResult<NotificationRecipientPage> {
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(AppError::invalid(
            "limit",
            "BASE_TYPE_BAD_LENGTH",
            "Must be between 1 and 200.",
        ));
    }
    let channel = ports
        .channels
        .find(channel_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Channel))?;
    let delivery_mode = effective_mode(ports, &channel).await?;
    let now = ports.clock.now();
    let mut page = NotificationRecipientPage {
        channel_id,
        guild_id: channel.guild_id,
        delivery_mode,
        evaluated_at: now,
        recipients: Vec::new(),
        next_after: None,
    };
    // Server-side mention accounting must not enumerate private-mode audiences.
    if delivery_mode != DeliveryMode::Standard {
        return Ok(page);
    }
    let (users, default) = if let Some(guild_id) = channel.guild_id {
        let guild = ports
            .guilds
            .find(guild_id)
            .await?
            .ok_or(AppError::NotFound(Resource::Guild))?;
        let roles = ports.guilds.roles(guild_id).await?;
        let parent = if channel.kind.is_thread() {
            let id = channel
                .parent_id
                .ok_or(AppError::NotFound(Resource::Channel))?;
            Some(
                ports
                    .channels
                    .find(id)
                    .await?
                    .ok_or(AppError::NotFound(Resource::Channel))?,
            )
        } else {
            None
        };
        let mut candidates = ports
            .guilds
            .member_profiles(guild_id, limit + 1, after)
            .await?;
        if candidates.len() > limit as usize {
            candidates.truncate(limit as usize);
            page.next_after = candidates.last().map(|member| member.member.user_id);
        }
        let access = GuildAccess::new(&guild, &roles, now);
        let users = candidates
            .into_iter()
            .map(|profile| profile.member)
            .filter(|member| {
                member.user_id != author_id
                    && access
                        .in_channel(member, &channel, parent.as_ref())
                        .contains(Permissions::VIEW_CHANNEL)
            })
            .map(|member| (member.user_id, member.role_ids))
            .collect();
        (
            users,
            notification_policy::guild_default(guild.default_message_notifications),
        )
    } else {
        let mut candidates: Vec<_> = channel
            .recipient_ids
            .iter()
            .copied()
            .filter(|id| after.is_none_or(|after| *id > after))
            .collect();
        candidates.sort();
        candidates.dedup();
        if candidates.len() > limit as usize {
            candidates.truncate(limit as usize);
            page.next_after = candidates.last().copied();
        }
        (
            candidates
                .into_iter()
                .filter(|id| *id != author_id)
                .map(|id| (id, Vec::new()))
                .collect(),
            NotifyLevel::AllMessages,
        )
    };
    page.recipients = notification_policy::resolve(ports, &channel, default, now, users).await?;
    Ok(page)
}
