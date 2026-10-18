use crate::context::{Actor, Ports};
use crate::error::AppResult;

pub async fn execute(ports: &Ports, actor: &Actor) -> AppResult<()> {
    actor.require_user()?;
    if let Some(session_id) = actor.session_id() {
        ports.sessions.revoke(session_id, ports.clock.now()).await?;
    }
    Ok(())
}
