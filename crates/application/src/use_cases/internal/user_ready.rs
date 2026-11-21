use domain::guild::{GuildSnapshot, MemberProfile};
use domain::notification::{GuildNotificationSettings, DIRECT_MESSAGES};
use domain::relationship::Relationship;
use domain::user::{User, UserSettings};
use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;
use crate::events::PrivateChannelView;
use crate::support::views::{guild_snapshot, private_view, require_user, SNAPSHOT_MEMBER_LIMIT};

#[derive(Debug, Clone)]
pub struct UserReady {
    pub user: User,
    pub guilds: Vec<GuildSnapshot>,
    pub private_channels: Vec<PrivateChannelView>,
    pub relationships: Vec<(Relationship, User)>,
    pub settings: UserSettings,
    /// Stored notification settings for the user's guilds and DMs.
    pub guild_settings: Vec<GuildNotificationSettings>,
    /// The user's private notes as `(target_id, note)`.
    pub notes: Vec<(Snowflake, String)>,
}

/// `GET /internal/v1/users/{id}/ready`: everything the gateway needs for READY.
pub async fn execute(ports: &Ports, user_id: Snowflake) -> AppResult<UserReady> {
    let user = require_user(ports, user_id).await?;
    let mut guilds = Vec::new();
    for guild in ports.guilds.list_for_user(user_id).await? {
        let mut snapshot = guild_snapshot(ports, guild.id).await?;
        // The capped member snapshot must still include the recipient, so a
        // pending member can resume screening after reconnecting to a large guild.
        if !snapshot
            .members
            .iter()
            .any(|profile| profile.member.user_id == user_id)
        {
            if let Some(member) = ports.guilds.member(guild.id, user_id).await? {
                if snapshot.members.len() >= SNAPSHOT_MEMBER_LIMIT as usize {
                    snapshot.members.pop();
                }
                snapshot.members.push(MemberProfile {
                    member,
                    user: user.clone(),
                });
            }
        }
        guilds.push(snapshot);
    }
    let mut private_channels = Vec::new();
    for channel in ports.channels.list_private(user_id).await? {
        private_channels.push(private_view(ports, channel).await?);
    }
    let relationships = ports.relationships.list(user_id).await?;
    let ids: Vec<_> = relationships.iter().map(|r| r.target_id).collect();
    let users = ports.users.find_many(&ids).await?;
    let relationships = relationships
        .into_iter()
        .filter_map(|r| {
            users
                .iter()
                .find(|u| u.id == r.target_id)
                .cloned()
                .map(|u| (r, u))
        })
        .collect();
    let settings = ports.users.settings(user_id).await?;
    let guild_settings = ports
        .users
        .list_guild_notification_settings(user_id)
        .await?
        .into_iter()
        .filter(|s| {
            s.guild_id == DIRECT_MESSAGES || guilds.iter().any(|g| g.guild.id == s.guild_id)
        })
        .collect();
    let notes = ports.users.notes(user_id).await?;
    Ok(UserReady {
        user,
        guilds,
        private_channels,
        relationships,
        settings,
        guild_settings,
        notes,
    })
}
