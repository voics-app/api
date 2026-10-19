use domain::user::UserSettings;

use crate::context::{Actor, Ports};
use crate::error::AppResult;

pub async fn execute(ports: &Ports, actor: &Actor) -> AppResult<UserSettings> {
    actor.require_user()?;
    ports.users.settings(actor.user_id).await
}
