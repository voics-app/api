use domain::channel::{Channel, ChannelType};
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::{AppError, AppResult, Resource};
use crate::support::guild_context::GuildContext;

/// Permissions a recipient holds in a DM or group DM.
pub const PRIVATE_CHANNEL_PERMISSIONS: Permissions = Permissions::from_bits(
    Permissions::VIEW_CHANNEL.bits()
        | Permissions::SEND_MESSAGES.bits()
        | Permissions::READ_MESSAGE_HISTORY.bits()
        | Permissions::ADD_REACTIONS.bits()
        | Permissions::EMBED_LINKS.bits()
        | Permissions::ATTACH_FILES.bits()
        | Permissions::USE_EXTERNAL_EMOJIS.bits()
        | Permissions::CONNECT.bits()
        | Permissions::SPEAK.bits()
        | Permissions::STREAM.bits()
        | Permissions::USE_VAD.bits()
        | Permissions::USE_APPLICATION_COMMANDS.bits(),
);

/// A channel together with what the caller may do in it.
#[derive(Debug, Clone)]
pub struct ChannelAccess {
    pub channel: Channel,
    pub guild: Option<GuildContext>,
    pub parent: Option<Channel>,
    pub permissions: Permissions,
}

impl ChannelAccess {
    /// Resolves the caller's permissions without requiring VIEW_CHANNEL.
    /// Fails with 50001 when the caller is neither member nor recipient.
    pub async fn resolve(
        ports: &Ports,
        channel_id: Snowflake,
        user_id: Snowflake,
    ) -> AppResult<Self> {
        let channel = ports
            .channels
            .find(channel_id)
            .await?
            .ok_or(AppError::NotFound(Resource::Channel))?;
        let Some(guild_id) = channel.guild_id else {
            if !channel.is_recipient(user_id) {
                return Err(AppError::missing_access());
            }
            return Ok(Self {
                channel,
                guild: None,
                parent: None,
                permissions: PRIVATE_CHANNEL_PERMISSIONS,
            });
        };
        let guild = GuildContext::load(ports, guild_id, user_id).await?;
        let parent = match (channel.kind.is_thread(), channel.parent_id) {
            (true, Some(parent_id)) => ports.channels.find(parent_id).await?,
            _ => None,
        };
        if parent.as_ref().is_some_and(|parent| {
            parent.kind == ChannelType::GuildForum && parent.delivery_mode != channel.delivery_mode
        }) {
            return Err(AppError::missing_access());
        }
        let permissions = guild.channel_permissions(&channel, parent.as_ref());
        Ok(Self {
            channel,
            guild: Some(guild),
            parent,
            permissions,
        })
    }

    /// Like [`Self::resolve`] but also requires VIEW_CHANNEL.
    pub async fn visible(
        ports: &Ports,
        channel_id: Snowflake,
        user_id: Snowflake,
    ) -> AppResult<Self> {
        let access = Self::resolve(ports, channel_id, user_id).await?;
        if access
            .channel
            .forum_settings
            .get("pending")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            return Err(AppError::NotFound(Resource::Channel));
        }
        if !access.permissions.contains(Permissions::VIEW_CHANNEL) {
            return Err(AppError::missing_access());
        }
        Ok(access)
    }

    pub fn require(&self, required: Permissions) -> AppResult<()> {
        if self.permissions.contains(required) {
            Ok(())
        } else {
            Err(AppError::missing_permissions())
        }
    }

    /// The guild context, or 50001-style failure for private channels.
    pub fn guild(&self) -> AppResult<&GuildContext> {
        self.guild.as_ref().ok_or_else(AppError::missing_access)
    }
}
