use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};

/// Removes the push token of the caller's session (the app does this before it signs out).
/// Removing a token that is not there is not an error.
pub async fn execute(ports: &Ports, actor: &Actor) -> AppResult<()> {
    actor.require_user()?;
    let session_id = actor.session_id().ok_or(AppError::Unauthorized)?;
    ports.push_tokens.delete_for_session(session_id).await?;
    Ok(())
}
