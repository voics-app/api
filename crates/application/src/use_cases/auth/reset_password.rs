use domain::account::TokenPurpose;
use domain::validation;

use crate::context::Ports;
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::account_mail;
use crate::support::sessions;
use crate::support::views::require_user;
use crate::write_set::{Mutation, WriteSet};

/// Sets a new password from an emailed token, signs the account out
/// everywhere and consumes the token. Two-factor stays on: a reset proves the
/// mailbox, not the authenticator.
pub async fn execute(ports: &Ports, token: &str, new_password: &str) -> AppResult<()> {
    validation::password(new_password)
        .map_err(|e| AppError::invalid("password", e.code, e.message))?;
    let now = ports.clock.now();
    let record = ports
        .account_tokens
        .consume(
            &ports.secrets.hash(token.trim()),
            TokenPurpose::ResetPassword,
            now,
        )
        .await?
        .ok_or_else(|| {
            AppError::invalid(
                "token",
                "INVALID_TOKEN",
                "The reset token is invalid or expired.",
            )
        })?;
    let mut user = require_user(ports, record.user_id).await?;
    user.verified = true;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::SetPasswordHash {
            user_id: user.id,
            hash: ports.passwords.hash(new_password).await?,
        })
        .mutate(Mutation::UpdateUser(user.clone()))
        .emit(DomainEvent::UserUpdate(user.clone()));
    ports.unit_of_work.commit(writes).await?;
    sessions::revoke(ports, user.id, None).await?;
    if let Some(address) = &user.email {
        account_mail::notify(
            ports,
            address,
            &format!("Your {} password was reset", account_mail::brand(ports)),
            &format!(
                "The password of your {} account was reset and every session was signed out.",
                account_mail::brand(ports)
            ),
        )
        .await;
    }
    Ok(())
}
