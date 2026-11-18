use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::events::DomainEvent;
use crate::support::account_mail;
use crate::support::credentials::{confirm_password, mfa_not_enabled, verify_second_factor};
use crate::support::views::require_user;
use crate::write_set::{Mutation, WriteSet};

/// Turns two-factor off. Needs the password and a TOTP or backup code, so a
/// stolen session alone cannot strip the second factor.
pub async fn execute(ports: &Ports, actor: &Actor, password: &str, code: &str) -> AppResult<()> {
    actor.require_user()?;
    let credentials = confirm_password(ports, actor.user_id, password).await?;
    if credentials.totp_secret.is_none() {
        return Err(mfa_not_enabled());
    }
    verify_second_factor(ports, actor.user_id, &credentials, code, "code").await?;
    let mut user = require_user(ports, actor.user_id).await?;
    user.mfa_enabled = !ports.passkeys.list(actor.user_id).await?.is_empty();
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::SetTotpSecret {
            user_id: actor.user_id,
            secret: None,
        })
        .mutate(Mutation::UpdateUser(user.clone()))
        .emit(DomainEvent::UserUpdate(user.clone()));
    ports.unit_of_work.commit(writes).await?;
    if let Some(address) = &user.email {
        account_mail::notify(
            ports,
            address,
            "Authenticator app removed",
            &format!(
                "The authenticator app was removed from your {} account. Any registered passkeys remain active.",
                account_mail::brand(ports)
            ),
        )
        .await;
    }
    Ok(())
}
