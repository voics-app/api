use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::ports::ActiveSession;

/// A live session of the caller, flagged when it is the one making the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionView {
    pub active: ActiveSession,
    pub current: bool,
}

/// `GET /users/@me/sessions`: the caller's live login sessions, most recently
/// used first.
pub async fn execute(ports: &Ports, actor: &Actor) -> AppResult<Vec<SessionView>> {
    actor.require_user()?;
    let current = actor.session_id();
    let sessions = ports
        .sessions
        .list_active(actor.user_id, ports.clock.now())
        .await?;
    Ok(sessions
        .into_iter()
        .map(|active| SessionView {
            current: Some(active.session.id) == current,
            active,
        })
        .collect())
}
