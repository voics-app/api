use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::support::sessions;

/// `POST /users/@me/sessions/logout-all`: revokes every session except the
/// caller's, so their tokens stop working and their gateway connections close.
pub async fn execute(ports: &Ports, actor: &Actor) -> AppResult<()> {
    actor.require_user()?;
    sessions::revoke(ports, actor.user_id, actor.session_id()).await
}
