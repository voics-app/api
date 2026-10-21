use crate::context::Ports;
use crate::error::AppResult;
use crate::support::account_mail::send_password_reset;

/// Emails a reset token when `email` belongs to an account. The outcome is the
/// same either way (and delivery failures are only logged) so the endpoint
/// cannot be used to find out which addresses are registered.
pub async fn execute(ports: &Ports, email: &str) -> AppResult<()> {
    let address = email.trim().to_lowercase();
    let Some(user) = ports.users.find_by_email(&address).await? else {
        return Ok(());
    };
    if user.bot {
        return Ok(());
    }
    if let Err(error) = send_password_reset(ports, user.id, &user.username, &address).await {
        tracing::warn!(%error, "password reset email not sent");
    }
    Ok(())
}
