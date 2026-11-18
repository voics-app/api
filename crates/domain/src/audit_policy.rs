use crate::audit::{AuditAction, AuditLogEntry};
use crate::validation::FieldError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditPolicy {
    pub enabled: bool,
    pub retention_days: u16,
}

impl Default for AuditPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_days: 90,
        }
    }
}

impl AuditPolicy {
    pub fn set_retention(&mut self, days: u16) -> Result<(), FieldError> {
        if ![30, 90, 365].contains(&days) {
            return Err(FieldError {
                field: "audit_log_retention_days".into(),
                code: "BASE_TYPE_CHOICES",
                message: "Choose 30, 90 or 365 days.".into(),
            });
        }
        self.retention_days = days;
        Ok(())
    }
}

impl AuditLogEntry {
    pub fn changes_audit_policy(&self) -> bool {
        self.action == AuditAction::GuildUpdate
            && self.changes.iter().any(|change| {
                matches!(
                    change.key.as_str(),
                    "audit_log_enabled" | "audit_log_retention_days"
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retention_has_explicit_supported_choices() {
        let mut policy = AuditPolicy::default();
        assert_eq!(policy.retention_days, 90);
        for days in [30, 90, 365] {
            assert!(policy.set_retention(days).is_ok());
        }
        for days in [0, 7, 31, 366, u16::MAX] {
            assert!(policy.set_retention(days).is_err());
        }
    }
}
