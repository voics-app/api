use crate::{AppResult, Ports};

/// Removes audit entries after their guild's retention window, even while
/// logging is disabled. Query filtering enforces the window between runs.
pub async fn execute(ports: &Ports) -> AppResult<u64> {
    ports.audit_log.prune_expired(ports.clock.now()).await
}
