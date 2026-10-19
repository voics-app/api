use domain::audit::{AuditAction, AuditChange, AuditLogEntry};
use domain::validation;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;

/// Who did what, with the optional `X-Audit-Log-Reason`.
#[derive(Debug, Clone, Copy)]
pub struct AuditContext<'a> {
    pub actor_id: Snowflake,
    pub reason: Option<&'a str>,
}

impl AuditContext<'_> {
    pub fn entry(
        &self,
        ports: &Ports,
        guild_id: Snowflake,
        action: AuditAction,
        target_id: Option<Snowflake>,
        changes: Vec<AuditChange>,
    ) -> AppResult<AuditLogEntry> {
        if let Some(reason) = self.reason {
            validation::audit_reason(reason)?;
        }
        Ok(AuditLogEntry {
            id: ports.ids.next_id(),
            guild_id,
            user_id: Some(self.actor_id),
            target_id,
            action,
            changes,
            reason: self.reason.map(str::to_owned),
            created_at: ports.clock.now(),
        })
    }
}
