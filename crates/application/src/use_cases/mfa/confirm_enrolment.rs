use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::account_mail;
use crate::support::throttle::{self, MFA_ATTEMPTS, MFA_WINDOW};
use crate::support::views::require_user;
use crate::write_set::{Mutation, WriteSet};

use super::new_backup_codes;

/// Activates two-factor once `code` matches the pending secret, and returns
/// the plaintext backup codes, which are never shown again.
pub async fn execute(ports: &Ports, actor: &Actor, code: &str) -> AppResult<Vec<String>> {
    actor.require_user()?;
    throttle::hit(
        ports,
        &format!("mfa:{}", actor.user_id.as_i64()),
        MFA_ATTEMPTS,
        MFA_WINDOW,
    )
    .await?;
    let credentials = ports
        .users
        .credentials(actor.user_id)
        .await?
        .ok_or(AppError::Unauthorized)?;
    if credentials.totp_secret.is_some() {
        return Err(AppError::BadRequest {
            code: 60001,
            message: "Two-factor is already enabled.".into(),
        });
    }
    let pending = credentials.pending_totp_secret.ok_or_else(|| {
        AppError::invalid(
            "code",
            "NO_PENDING_ENROLMENT",
            "Start enrolment before confirming a code.",
        )
    })?;
    if !ports.totp.verify(&pending, code.trim(), ports.clock.now()) {
        return Err(AppError::invalid(
            "code",
            "INVALID_TOTP",
            "Invalid two-factor code.",
        ));
    }
    let codes = new_backup_codes(ports).await?;
    let mut user = require_user(ports, actor.user_id).await?;
    user.mfa_enabled = true;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::SetTotpSecret {
            user_id: actor.user_id,
            secret: Some(pending),
        })
        .mutate(Mutation::ReplaceBackupCodes {
            user_id: actor.user_id,
            hashes: codes.hashes,
        })
        .mutate(Mutation::UpdateUser(user.clone()))
        .emit(DomainEvent::UserUpdate(user.clone()));
    ports.unit_of_work.commit(writes).await?;
    if let Some(address) = &user.email {
        account_mail::notify(
            ports,
            address,
            "Two-factor authentication enabled",
            "Two-factor authentication was turned on for your Discordy account.",
        )
        .await;
    }
    Ok(codes.display)
}
