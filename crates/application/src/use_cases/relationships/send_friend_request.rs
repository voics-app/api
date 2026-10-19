use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::use_cases::relationships::put_relationship;

/// `POST /users/@me/relationships {username}`.
pub async fn execute(ports: &Ports, actor: &Actor, username: &str) -> AppResult<()> {
    let target = ports
        .users
        .find_by_username(username.trim())
        .await?
        .ok_or(AppError::NotFound(Resource::User))?;
    put_relationship::to_user(ports, actor, &target, None).await
}
