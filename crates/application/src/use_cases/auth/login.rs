use domain::account::AccountState;
use domain::session::ClientInfo;
use domain::user::User;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::{AppError, AppResult, Denial};
use crate::support::account_mail::issue_mfa_ticket;
use crate::support::credentials::verify_second_factor;
use crate::use_cases::auth::tokens::{start_session, AuthTokens};
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone)]
pub struct LoginInput {
    pub email: String,
    pub password: String,
    /// A TOTP code or an unused backup code.
    pub totp: Option<String>,
    /// Binds the session to one of the caller's registered devices.
    pub device_id: Option<Snowflake>,
    pub client: ClientInfo,
}

fn invalid_login() -> AppError {
    AppError::invalid("login", "INVALID_LOGIN", "Login or password is invalid.")
}

pub async fn execute(ports: &Ports, input: LoginInput) -> AppResult<AuthTokens> {
    let email = input.email.trim().to_lowercase();
    let user = ports
        .users
        .find_by_email(&email)
        .await?
        .ok_or_else(invalid_login)?;
    let credentials = ports
        .users
        .credentials(user.id)
        .await?
        .ok_or_else(invalid_login)?;
    if !ports
        .passwords
        .verify(&input.password, &credentials.password_hash)
        .await?
    {
        return Err(invalid_login());
    }
    if user.is_suspended(ports.clock.now()) {
        return Err(AppError::Forbidden(Denial::Suspended));
    }
    let passkeys = ports.passkeys.list(user.id).await?;
    if credentials.totp_secret.is_some() || !passkeys.is_empty() {
        let Some(code) = input.totp.as_deref() else {
            let ticket = issue_mfa_ticket(ports, user.id).await?;
            let mut methods = Vec::new();
            if credentials.totp_secret.is_some() {
                methods.push("totp");
            }
            if !passkeys.is_empty() && ports.webauthn.is_some() {
                methods.push("webauthn");
            }
            methods.push("backup_code");
            return Err(AppError::MfaRequired { ticket, methods });
        };
        verify_second_factor(ports, user.id, &credentials, code, "totp").await?;
    }
    complete(ports, user, input.device_id, input.client).await
}

/// Final step shared by every login path once the caller is authenticated:
/// reactivates a disabled or deletion-scheduled account, checks the device
/// and starts the session.
pub(super) async fn complete(
    ports: &Ports,
    user: User,
    device_id: Option<Snowflake>,
    client: ClientInfo,
) -> AppResult<AuthTokens> {
    if let Some(device_id) = device_id {
        let owned = ports
            .devices
            .find(device_id)
            .await?
            .is_some_and(|d| d.user_id == user.id);
        if !owned {
            return Err(AppError::invalid(
                "device_id",
                "UNKNOWN_DEVICE",
                "Unknown device.",
            ));
        }
    }
    if ports.users.account_state(user.id).await?.is_inactive() {
        let mut writes = WriteSet::new();
        writes.mutate(Mutation::SetAccountState {
            user_id: user.id,
            state: AccountState::default(),
        });
        ports.unit_of_work.commit(writes).await?;
    }
    start_session(ports, user, device_id, client).await
}
