use domain::account::TokenPurpose;
use domain::session::ClientInfo;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::{AppError, AppResult, Denial};
use crate::support::credentials::verify_second_factor;
use crate::support::views::require_user;
use crate::use_cases::auth::login::complete;
use crate::use_cases::auth::tokens::AuthTokens;

#[derive(Debug, Clone)]
pub struct MfaLoginInput {
    /// Ticket from the 60008 answer of `login`.
    pub ticket: String,
    /// A TOTP code or an unused backup code.
    pub code: String,
    pub device_id: Option<Snowflake>,
    pub client: ClientInfo,
}

fn invalid_ticket() -> AppError {
    AppError::invalid(
        "ticket",
        "INVALID_MFA_TICKET",
        "The two-factor ticket is invalid or expired. Log in again.",
    )
}

/// Completes a login whose password step passed. Wrong codes count against the
/// user's second-factor throttle; the ticket is consumed only on success.
pub async fn execute(ports: &Ports, input: MfaLoginInput) -> AppResult<AuthTokens> {
    let now = ports.clock.now();
    let ticket_hash = ports.secrets.hash(&input.ticket);
    let ticket = ports
        .account_tokens
        .find_active(&ticket_hash, TokenPurpose::MfaLogin, now)
        .await?
        .ok_or_else(invalid_ticket)?;
    let user = require_user(ports, ticket.user_id).await?;
    if user.is_suspended(now) {
        return Err(AppError::Forbidden(Denial::Suspended));
    }
    let credentials = ports
        .users
        .credentials(user.id)
        .await?
        .ok_or_else(invalid_ticket)?;
    verify_second_factor(ports, user.id, &credentials, &input.code, "code").await?;
    ports
        .account_tokens
        .consume(&ticket_hash, TokenPurpose::MfaLogin, now)
        .await?
        .ok_or_else(invalid_ticket)?;
    complete(ports, user, input.device_id, input.client).await
}
