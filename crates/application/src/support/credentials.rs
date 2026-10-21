//! Re-authentication shared by sensitive account operations and login.

use domain::mfa::{looks_like_totp, normalize_backup_code};
use domain::user::Credentials;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::{AppError, AppResult};
use crate::support::throttle::{
    self, MFA_ATTEMPTS, MFA_WINDOW, PASSWORD_ATTEMPTS, PASSWORD_WINDOW,
};

pub fn wrong_password() -> AppError {
    AppError::invalid(
        "password",
        "PASSWORD_DOES_NOT_MATCH",
        "Password does not match.",
    )
}

pub fn invalid_code(field: &str) -> AppError {
    AppError::invalid(field, "INVALID_TOTP", "Invalid two-factor code.")
}

pub fn mfa_not_enabled() -> AppError {
    AppError::BadRequest {
        code: 60002,
        message: "Two-factor is not enabled.".into(),
    }
}

/// Checks the caller's current password (throttled) and returns their credentials.
pub async fn confirm_password(
    ports: &Ports,
    user_id: Snowflake,
    password: &str,
) -> AppResult<Credentials> {
    throttle::hit(
        ports,
        &format!("pw:{}", user_id.as_i64()),
        PASSWORD_ATTEMPTS,
        PASSWORD_WINDOW,
    )
    .await?;
    let credentials = ports
        .users
        .credentials(user_id)
        .await?
        .ok_or(AppError::Unauthorized)?;
    if !ports
        .passwords
        .verify(password, &credentials.password_hash)
        .await?
    {
        return Err(wrong_password());
    }
    Ok(credentials)
}

/// Accepts a TOTP code (within the clock-skew window) or an unused backup
/// code, which it consumes. Attempts are throttled per user so wrong guesses
/// lock the account out of the second factor for a while.
pub async fn verify_second_factor(
    ports: &Ports,
    user_id: Snowflake,
    credentials: &Credentials,
    code: &str,
    field: &str,
) -> AppResult<()> {
    throttle::hit(
        ports,
        &format!("mfa:{}", user_id.as_i64()),
        MFA_ATTEMPTS,
        MFA_WINDOW,
    )
    .await?;
    let secret = credentials
        .totp_secret
        .as_deref()
        .ok_or_else(mfa_not_enabled)?;
    if looks_like_totp(code) {
        if ports.totp.verify(secret, code.trim(), ports.clock.now()) {
            return Ok(());
        }
    } else if let Some(canonical) = normalize_backup_code(code) {
        if consume_backup_code(ports, user_id, &canonical).await? {
            return Ok(());
        }
    }
    Err(invalid_code(field))
}

async fn consume_backup_code(
    ports: &Ports,
    user_id: Snowflake,
    canonical: &str,
) -> AppResult<bool> {
    for record in ports.users.backup_codes(user_id).await? {
        if record.consumed || !ports.passwords.verify(canonical, &record.hash).await? {
            continue;
        }
        return ports
            .users
            .consume_backup_code(record.id, ports.clock.now())
            .await;
    }
    Ok(false)
}
