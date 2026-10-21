use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::support::account_mail::send_verification;
use crate::support::views::require_user;

/// Sends the caller's unverified address a fresh verification token.
/// Rate limited per recipient; the error reaches the caller.
pub async fn execute(ports: &Ports, actor: &Actor) -> AppResult<()> {
    actor.require_user()?;
    let user = require_user(ports, actor.user_id).await?;
    let Some(address) = user.email.as_deref().filter(|_| !user.verified) else {
        return Err(AppError::invalid(
            "email",
            "ALREADY_VERIFIED",
            "Email is already verified.",
        ));
    };
    send_verification(ports, user.id, &user.username, address, false).await
}
