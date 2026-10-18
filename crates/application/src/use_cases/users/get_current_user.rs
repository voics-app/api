use domain::user::User;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::support::views::require_user;

pub async fn execute(ports: &Ports, actor: &Actor) -> AppResult<User> {
    require_user(ports, actor.user_id).await
}
