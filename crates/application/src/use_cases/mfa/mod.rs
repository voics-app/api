//! TOTP two-factor enrolment and backup codes.

pub mod backup_codes;
pub mod begin_enrolment;
pub mod confirm_enrolment;
pub mod disable;

use domain::mfa::{format_backup_code, BACKUP_CODE_COUNT};

use crate::context::Ports;
use crate::error::AppResult;

/// A freshly generated code set: what the user sees, and what is stored.
pub(crate) struct NewBackupCodes {
    pub display: Vec<String>,
    pub hashes: Vec<String>,
}

pub(crate) async fn new_backup_codes(ports: &Ports) -> AppResult<NewBackupCodes> {
    let mut display = Vec::with_capacity(BACKUP_CODE_COUNT);
    let mut hashes = Vec::with_capacity(BACKUP_CODE_COUNT);
    for _ in 0..BACKUP_CODE_COUNT {
        let canonical = ports.secrets.backup_code();
        hashes.push(ports.passwords.hash(&canonical).await?);
        display.push(format_backup_code(&canonical));
    }
    Ok(NewBackupCodes { display, hashes })
}
