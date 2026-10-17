use chrono::{DateTime, Utc};

use domain::access::GuildAccess;
use domain::channel::Channel;
use domain::guild::{Guild, Member, Role};
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::{AppError, AppResult, Resource};

/// A guild seen through the eyes of one of its members.
#[derive(Debug, Clone)]
pub struct GuildContext {
    pub guild: Guild,
    pub roles: Vec<Role>,
    pub member: Member,
    pub now: DateTime<Utc>,
}

impl GuildContext {
    /// Loads the guild and the caller's membership (50001 if not a member).
    pub async fn load(ports: &Ports, guild_id: Snowflake, user_id: Snowflake) -> AppResult<Self> {
        let guild = ports.guilds.find(guild_id).await?.ok_or(AppError::NotFound(Resource::Guild))?;
        let member = ports
            .guilds
            .member(guild_id, user_id)
            .await?
            .ok_or_else(AppError::missing_access)?;
        let roles = ports.guilds.roles(guild_id).await?;
        Ok(Self { guild, roles, member, now: ports.clock.now() })
    }

    pub fn access(&self) -> GuildAccess<'_> {
        GuildAccess::new(&self.guild, &self.roles, self.now)
    }

    pub fn is_owner(&self) -> bool {
        self.guild.owner_id == self.member.user_id
    }

    pub fn permissions(&self) -> Permissions {
        self.access().base(&self.member)
    }

    pub fn require(&self, required: Permissions) -> AppResult<()> {
        if self.permissions().contains(required) {
            Ok(())
        } else {
            Err(AppError::missing_permissions())
        }
    }

    pub fn channel_permissions(&self, channel: &Channel, parent: Option<&Channel>) -> Permissions {
        self.access().in_channel(&self.member, channel, parent)
    }

    pub fn require_in_channel(&self, channel: &Channel, parent: Option<&Channel>, required: Permissions) -> AppResult<()> {
        let permissions = self.channel_permissions(channel, parent);
        if !permissions.contains(Permissions::VIEW_CHANNEL) {
            return Err(AppError::missing_access());
        }
        if !permissions.contains(required) {
            return Err(AppError::missing_permissions());
        }
        Ok(())
    }

    /// The caller may only grant permissions they hold themselves.
    pub fn require_grantable(&self, permissions: Permissions) -> AppResult<()> {
        if self.permissions().contains(permissions) {
            Ok(())
        } else {
            Err(AppError::missing_permissions())
        }
    }

    pub fn require_outranks(&self, target: &Member) -> AppResult<()> {
        if self.access().outranks(&self.member, target) {
            Ok(())
        } else {
            Err(AppError::missing_permissions())
        }
    }

    pub fn require_manageable_role(&self, role: &Role) -> AppResult<()> {
        if self.access().can_manage_role(&self.member, role) {
            Ok(())
        } else {
            Err(AppError::missing_permissions())
        }
    }

    pub fn role(&self, role_id: Snowflake) -> AppResult<&Role> {
        self.roles.iter().find(|r| r.id == role_id).ok_or(AppError::NotFound(Resource::Role))
    }

    pub fn highest_position(&self) -> i32 {
        self.access().highest_position(&self.member)
    }
}
