use domain::application::Application;
use domain::user::User;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::support::views::require_user;

/// An application with its bot user.
#[derive(Debug, Clone)]
pub struct ApplicationView {
    pub application: Application,
    pub bot: User,
    pub owner: User,
}

pub async fn view(ports: &Ports, application: Application) -> AppResult<ApplicationView> {
    let bot = require_user(ports, application.bot_user_id).await?;
    let owner = require_user(ports, application.owner_id).await?;
    Ok(ApplicationView { application, bot, owner })
}

/// The owner (user token) or the application's own bot may manage it.
pub async fn require_manager(ports: &Ports, actor: &Actor, application_id: Snowflake) -> AppResult<Application> {
    let application = ports
        .applications
        .find(application_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Application))?;
    let allowed = match actor.application_id() {
        Some(own) => own == application.id,
        None => actor.user_id == application.owner_id,
    };
    if !allowed {
        return Err(AppError::NotFound(Resource::Application));
    }
    Ok(application)
}

/// Only the human owner (not the bot) may perform this.
pub async fn require_owner(ports: &Ports, actor: &Actor, application_id: Snowflake) -> AppResult<Application> {
    actor.require_user()?;
    require_manager(ports, actor, application_id).await
}
