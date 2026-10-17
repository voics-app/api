use domain::Snowflake;

use crate::context::Ports;
use crate::error::{AppError, AppResult, Denial};
use crate::use_cases::auth::tokens::{start_session, AuthTokens};

#[derive(Debug, Clone)]
pub struct LoginInput {
    pub email: String,
    pub password: String,
    pub totp: Option<String>,
    /// Binds the session to one of the caller's registered devices.
    pub device_id: Option<Snowflake>,
}

fn invalid_login() -> AppError {
    AppError::invalid("login", "INVALID_LOGIN", "Login or password is invalid.")
}

pub async fn execute(ports: &Ports, input: LoginInput) -> AppResult<AuthTokens> {
    let email = input.email.trim().to_lowercase();
    let user = ports.users.find_by_email(&email).await?.ok_or_else(invalid_login)?;
    let credentials = ports.users.credentials(user.id).await?.ok_or_else(invalid_login)?;
    if !ports.passwords.verify(&input.password, &credentials.password_hash).await? {
        return Err(invalid_login());
    }
    if user.is_suspended(ports.clock.now()) {
        return Err(AppError::Forbidden(Denial::Suspended));
    }
    if let Some(secret) = &credentials.totp_secret {
        let code = input.totp.as_deref().ok_or(AppError::MfaRequired)?;
        if !ports.totp.verify(secret, code, ports.clock.now()) {
            return Err(AppError::invalid("totp", "INVALID_TOTP", "Invalid two-factor code."));
        }
    }
    if let Some(device_id) = input.device_id {
        let owned = ports.devices.find(device_id).await?.is_some_and(|d| d.user_id == user.id);
        if !owned {
            return Err(AppError::invalid("device_id", "UNKNOWN_DEVICE", "Unknown device."));
        }
    }
    start_session(ports, user, input.device_id).await
}
