use domain::delivery_mode::DeliveryMode;
use domain::user::{CustomStatus, PresenceStatus, Theme, UserSettings};

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
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpsertSettings(settings.clone()))
        .emit(DomainEvent::UserSettingsUpdate(settings.clone()));
    ports.unit_of_work.commit(writes).await?;
    Ok(settings)
}
