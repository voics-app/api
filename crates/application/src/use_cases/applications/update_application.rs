use domain::application::application_flags;
use domain::validation;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::images::{store_hashed, ImageUpload};
use crate::support::views::require_user;
use crate::use_cases::applications::app_access::{require_manager, view, ApplicationView};
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone, Default)]
pub struct UpdateApplicationInput {
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<Option<ImageUpload>>,
    pub bot_public: Option<bool>,
    pub bot_require_code_grant: Option<bool>,
    pub flags: Option<u64>,
    pub interactions_endpoint_url: Option<Option<String>>,
}

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    application_id: Snowflake,
    input: UpdateApplicationInput,
) -> AppResult<ApplicationView> {
    let mut application = require_manager(ports, actor, application_id).await?;
    let mut writes = WriteSet::new();
    if let Some(name) = input.name {
        validation::application_name(&name)?;
        application.name = name.trim().to_owned();
        let mut bot = require_user(ports, application.bot_user_id).await?;
        bot.global_name = Some(application.name.clone());
        writes.mutate(Mutation::UpdateUser(bot.clone())).emit(DomainEvent::UserUpdate(bot));
    }
    if let Some(description) = input.description {
        if description.chars().count() > 400 {
            return Err(AppError::invalid("description", "BASE_TYPE_BAD_LENGTH", "Must be at most 400 characters."));
        }
        application.description = description;
    }
    if let Some(public) = input.bot_public {
        application.bot_public = public;
    }
    if let Some(require_code) = input.bot_require_code_grant {
        application.bot_require_code_grant = require_code;
    }
    if let Some(flags) = input.flags {
        if flags & !application_flags::USER_EDITABLE != 0 {
            return Err(AppError::invalid("flags", "APPLICATION_FLAGS_INVALID", "Only *_LIMITED intent flags are editable."));
        }
        application.flags = flags;
    }
    if let Some(url) = input.interactions_endpoint_url {
        if url.as_ref().is_some_and(|u| !u.starts_with("https://")) {
            return Err(AppError::invalid("interactions_endpoint_url", "URL_TYPE_INVALID_URL", "Must be an https URL."));
        }
        application.interactions_endpoint_url = url;
    }
    if let Some(icon) = input.icon {
        application.icon = match icon {
            Some(image) => Some(store_hashed(ports, &format!("app-icons/{application_id}"), image).await?),
            None => None,
        };
    }
    writes.mutate(Mutation::UpdateApplication(application.clone()));
    ports.unit_of_work.commit(writes).await?;
    view(ports, application).await
}
