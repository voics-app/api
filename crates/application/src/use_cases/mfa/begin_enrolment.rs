use domain::mfa::otpauth_uri;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::support::credentials::confirm_password;
use crate::support::views::require_user;
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TotpEnrolment {
    pub secret: String,
    pub otpauth_uri: String,
}

/// Generates a pending TOTP secret. Two-factor stays off until
/// [`super::confirm_enrolment`] proves the authenticator works; calling this
/// again replaces the pending secret.
pub async fn execute(ports: &Ports, actor: &Actor, password: &str) -> AppResult<TotpEnrolment> {
    actor.require_user()?;
    let credentials = confirm_password(ports, actor.user_id, password).await?;
    if credentials.totp_secret.is_some() {
        return Err(AppError::BadRequest {
            code: 60001,
            message: "Two-factor is already enabled.".into(),
        });
    }
    let user = require_user(ports, actor.user_id).await?;
    let secret = ports.totp.generate_secret();
    let label = user.email.as_deref().unwrap_or(&user.username);
    let enrolment = TotpEnrolment {
        otpauth_uri: otpauth_uri(&ports.settings.brand_name, label, &secret),
        secret: secret.clone(),
    };
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::SetPendingTotp {
        user_id: actor.user_id,
        secret: Some(secret),
    });
    ports.unit_of_work.commit(writes).await?;
    Ok(enrolment)
}
