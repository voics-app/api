use domain::account::TokenPurpose;
use domain::user::User;
use domain::validation::ValidationErrors;

use crate::context::Ports;
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::views::require_user;
use crate::write_set::{Mutation, WriteSet};

fn invalid_token() -> AppError {
    AppError::invalid(
        "token",
        "INVALID_TOKEN",
        "The verification token is invalid or expired.",
    )
}

/// Marks the account verified. A token issued for an email change also
/// switches the account to the new address, unless someone registered it first.
pub async fn execute(ports: &Ports, token: &str) -> AppResult<User> {
    let now = ports.clock.now();
    let record = ports
        .account_tokens
        .consume(
            &ports.secrets.hash(token.trim()),
            TokenPurpose::VerifyEmail,
            now,
        )
        .await?
        .ok_or_else(invalid_token)?;
    let mut user = require_user(ports, record.user_id).await?;
    if let Some(email) = record.email {
        let taken = ports
            .users
            .find_by_email(&email)
            .await?
            .is_some_and(|owner| owner.id != user.id);
        if taken {
            return Err(ValidationErrors::single(
                "email",
                "EMAIL_ALREADY_REGISTERED",
                "Email is already registered.",
            )
            .into());
        }
        user.email = Some(email);
    }
    user.verified = true;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpdateUser(user.clone()))
        .emit(DomainEvent::UserUpdate(user.clone()));
    ports.unit_of_work.commit(writes).await?;
    require_user(ports, user.id).await
}
