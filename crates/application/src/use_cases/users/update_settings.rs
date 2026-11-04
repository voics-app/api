use domain::delivery_mode::DeliveryMode;
use domain::pinned::{self, PinnedChannels};
use domain::snowflake::Snowflake;
use domain::user::{CustomStatus, GuildFolder, PresenceStatus, Theme, UserSettings};

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone, Default)]
pub struct UpdateSettingsInput {
    pub default_delivery_mode: Option<String>,
    pub e2ee_calls_default: Option<bool>,
    pub theme: Option<String>,
    pub locale: Option<String>,
    pub status: Option<String>,
    pub custom_status: Option<Option<CustomStatus>>,
    pub developer_mode: Option<bool>,
    pub accent_color: Option<Option<u32>>,
    pub guild_folders: Option<Vec<GuildFolder>>,
    pub guild_positions: Option<Vec<Snowflake>>,
    pub pinned_dm_channel_ids: Option<Vec<Snowflake>>,
    pub pinned_channel_ids: Option<PinnedChannels>,
    pub message_display_compact: Option<bool>,
    /// Partial preferences object, merged key by key.
    pub preferences: Option<serde_json::Value>,
}

const MAX_RAIL_ENTRIES: usize = 200;
const MAX_FOLDER_NAME: usize = 32;

fn validate_folders(folders: &[GuildFolder]) -> AppResult<()> {
    let bad =
        |message: &'static str| AppError::invalid("guild_folders", "BASE_TYPE_BAD_LENGTH", message);
    if folders.len() > MAX_RAIL_ENTRIES {
        return Err(bad("Too many folders."));
    }
    let mut seen = std::collections::HashSet::new();
    let mut folder_ids = std::collections::HashSet::new();
    for folder in folders {
        if folder.guild_ids.is_empty() || folder.guild_ids.len() > MAX_RAIL_ENTRIES {
            return Err(bad("A folder holds between 1 and 200 servers."));
        }
        if folder.id.is_none() && folder.guild_ids.len() != 1 {
            return Err(bad("An entry without an id holds exactly one server."));
        }
        if folder
            .name
            .as_ref()
            .is_some_and(|n| n.chars().count() > MAX_FOLDER_NAME)
        {
            return Err(bad("Folder name must be at most 32 characters."));
        }
        if folder.id.is_some_and(|id| !folder_ids.insert(id)) {
            return Err(bad("Duplicate folder id."));
        }
        for guild in &folder.guild_ids {
            if !seen.insert(*guild) {
                return Err(bad("A server can only be in one place."));
            }
        }
    }
    Ok(())
}

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    input: UpdateSettingsInput,
) -> AppResult<UserSettings> {
    actor.require_user()?;
    let mut settings = ports.users.settings(actor.user_id).await?;
    if let Some(mode) = input.default_delivery_mode {
        settings.default_delivery_mode = mode.parse::<DeliveryMode>()?;
    }
    if let Some(e2ee) = input.e2ee_calls_default {
        settings.e2ee_calls_default = e2ee;
    }
    if let Some(theme) = input.theme {
        settings.theme = Theme::parse(&theme).ok_or_else(|| {
            AppError::invalid("theme", "BASE_TYPE_CHOICES", "Must be dark, oled or light.")
        })?;
    }
    if let Some(status) = input.status {
        settings.status = PresenceStatus::parse(&status).ok_or_else(|| {
            AppError::invalid(
                "status",
                "BASE_TYPE_CHOICES",
                "Must be online, idle, dnd or invisible.",
            )
        })?;
    }
    if let Some(locale) = input.locale {
        if locale.is_empty() || locale.len() > 16 {
            return Err(AppError::invalid(
                "locale",
                "BASE_TYPE_BAD_LENGTH",
                "Invalid locale.",
            ));
        }
        settings.locale = locale;
    }
    if let Some(custom_status) = input.custom_status {
        if custom_status
            .as_ref()
            .and_then(|s| s.text.as_ref())
            .is_some_and(|t| t.chars().count() > 128)
        {
            return Err(AppError::invalid(
                "custom_status",
                "BASE_TYPE_BAD_LENGTH",
                "Must be at most 128 characters.",
            ));
        }
        settings.custom_status = custom_status;
    }
    if let Some(developer_mode) = input.developer_mode {
        settings.developer_mode = developer_mode;
    }
    if let Some(accent_color) = input.accent_color {
        settings.accent_color = accent_color.map(|c| c & 0x00FF_FFFF);
    }
    if let Some(mut folders) = input.guild_folders {
        validate_folders(&folders)?;
        for folder in &mut folders {
            folder.color = folder.color.map(|c| c & 0x00FF_FFFF);
        }
        settings.guild_folders = folders;
    }
    if let Some(positions) = input.guild_positions {
        if positions.len() > MAX_RAIL_ENTRIES {
            return Err(AppError::invalid(
                "guild_positions",
                "BASE_TYPE_BAD_LENGTH",
                "Too many servers.",
            ));
        }
        settings.guild_positions = positions;
    }
    if let Some(pinned_dms) = input.pinned_dm_channel_ids {
        pinned::validate_pinned_dms(&pinned_dms).map_err(|e| {
            AppError::invalid("pinned_dm_channel_ids", "BASE_TYPE_BAD_LENGTH", e.message())
        })?;
        settings.pinned_dm_channel_ids = pinned_dms;
    }
    if let Some(pinned_channels) = input.pinned_channel_ids {
        settings.pinned_channel_ids =
            pinned::normalize_pinned_channels(pinned_channels).map_err(|e| {
                AppError::invalid("pinned_channel_ids", "BASE_TYPE_BAD_LENGTH", e.message())
            })?;
    }
    if let Some(compact) = input.message_display_compact {
        settings.message_display_compact = compact;
    }
    if let Some(patch) = input.preferences {
        settings.preferences = domain::user_preferences::merge(&settings.preferences, &patch)
            .map_err(|e| {
                AppError::invalid(
                    &format!("preferences.{}", e.key),
                    "BASE_TYPE_CHOICES",
                    e.message,
                )
            })?;
    }
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpsertSettings(settings.clone()))
        .emit(DomainEvent::UserSettingsUpdate(settings.clone()));
    ports.unit_of_work.commit(writes).await?;
    Ok(settings)
}
