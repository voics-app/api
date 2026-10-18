use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::views::require_user;
use crate::write_set::{Mutation, WriteSet};

pub async fn execute(ports: &Ports, actor: &Actor, code: &str) -> AppResult<()> {
    actor.require_user()?;
    let credentials = ports
        .users
        .credentials(actor.user_id)
        .await?
        .ok_or(AppError::Unauthorized)?;
    let secret = credentials.totp_secret.ok_or(AppError::BadRequest {
        code: 60002,
        message: "Two-factor is not enabled.".into(),
    })?;
    if !ports.totp.verify(&secret, code, ports.clock.now()) {
        return Err(AppError::invalid(
            "code",
            "INVALID_TOTP",
            "Invalid two-factor code.",
        ));
    }
    let mut user = require_user(ports, actor.user_id).await?;
    user.mfa_enabled = false;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::SetTotpSecret {
            user_id: actor.user_id,
            secret: None,
        })
        .mutate(Mutation::UpdateUser(user.clone()))
        .emit(DomainEvent::UserUpdate(user));
    ports.unit_of_work.commit(writes).await
}
