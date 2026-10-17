use domain::guild::GuildSnapshot;
use domain::relationship::Relationship;
use domain::user::{User, UserSettings};
use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;
use crate::events::PrivateChannelView;
use crate::support::views::{guild_snapshot, private_view, require_user};

#[derive(Debug, Clone)]
pub struct UserReady {
    pub user: User,
    pub guilds: Vec<GuildSnapshot>,
    pub private_channels: Vec<PrivateChannelView>,
    pub relationships: Vec<(Relationship, User)>,
    pub settings: UserSettings,
}

/// `GET /internal/v1/users/{id}/ready`: everything the gateway needs for READY.
pub async fn execute(ports: &Ports, user_id: Snowflake) -> AppResult<UserReady> {
    let user = require_user(ports, user_id).await?;
    let mut guilds = Vec::new();
    for guild in ports.guilds.list_for_user(user_id).await? {
        guilds.push(guild_snapshot(ports, guild.id).await?);
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
        .filter_map(|r| users.iter().find(|u| u.id == r.target_id).cloned().map(|u| (r, u)))
        .collect();
    let settings = ports.users.settings(user_id).await?;
    Ok(UserReady { user, guilds, private_channels, relationships, settings })
}
