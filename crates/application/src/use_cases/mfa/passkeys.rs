//! Account-bound registration; private WebAuthn state never crosses the HTTP boundary.

use chrono::Duration;
use serde_json::Value;
use uuid::Uuid;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::ports::{
    ChallengeKind, PasskeyChallenge, PasskeyRecord, WebauthnVerifier, CHALLENGE_LIFETIME_SECONDS,
    MAX_PASSKEYS,
};
use crate::support::account_mail;
use crate::support::credentials::{confirm_password, verify_second_factor};
use crate::support::views::require_user;

use super::{backup_codes::BackupCodeView, new_backup_codes};

pub struct Challenge {
    pub id: Uuid,
    pub public_key: Value,
}

pub struct Registration {
    pub passkey: PasskeyRecord,
    pub backup_codes: Option<Vec<BackupCodeView>>,
}

pub(crate) fn verifier(ports: &Ports) -> AppResult<&dyn WebauthnVerifier> {
    ports
        .webauthn
        .as_deref()
        .ok_or_else(|| AppError::Unavailable("Passkeys are not configured".into()))
}

pub(crate) fn invalid_challenge() -> AppError {
    AppError::invalid(
        "challenge_id",
        "INVALID_WEBAUTHN_CHALLENGE",
        "The passkey challenge is invalid, expired or already used. Start again.",
    )
}

fn session_binding(actor: &Actor) -> AppResult<String> {
    actor.require_user()?;
    Ok(format!(
        "{}:{}",
        actor.user_id,
        actor.session_id().ok_or(AppError::Unauthorized)?
    ))
}

pub async fn list(ports: &Ports, actor: &Actor) -> AppResult<Vec<PasskeyRecord>> {
    actor.require_user()?;
    ports.passkeys.list(actor.user_id).await
}

fn passkey_name(name: &str) -> AppResult<&str> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(AppError::invalid(
            "name",
            "INVALID_PASSKEY_NAME",
            "Use a name between 1 and 64 characters.",
        ));
    }
    Ok(name)
}

pub async fn rename(
    ports: &Ports,
    actor: &Actor,
    id: Uuid,
    name: &str,
) -> AppResult<PasskeyRecord> {
    actor.require_user()?;
    ports
        .passkeys
        .rename(actor.user_id, id, passkey_name(name)?)
        .await?
        .ok_or_else(|| AppError::BadRequest {
            code: 50035,
            message: "Unknown passkey.".into(),
        })
}

pub async fn start(
    ports: &Ports,
    actor: &Actor,
    password: &str,
    name: &str,
) -> AppResult<Challenge> {
    let binding = session_binding(actor)?;
    let verifier = verifier(ports)?;
    let name = passkey_name(name)?;
    confirm_password(ports, actor.user_id, password).await?;
    let user = require_user(ports, actor.user_id).await?;
    let keys = ports.passkeys.list(user.id).await?;
    if keys.len() >= MAX_PASSKEYS {
        return Err(AppError::invalid(
            "name",
            "PASSKEY_LIMIT",
            "At most ten passkeys may be registered.",
        ));
    }
    let ceremony = verifier.start_registration(&user, &keys)?;
    let now = ports.clock.now();
    let challenge = PasskeyChallenge {
        id: Uuid::now_v7(),
        user_id: user.id,
        kind: ChallengeKind::Registration,
        binding,
        name: Some(name.into()),
        state: ceremony.state,
        expires_at: now + Duration::seconds(CHALLENGE_LIFETIME_SECONDS),
    };
    ports.passkeys.create_challenge(&challenge, now).await?;
    Ok(Challenge {
        id: challenge.id,
        public_key: ceremony.public_key,
    })
}

pub async fn finish(
    ports: &Ports,
    actor: &Actor,
    challenge_id: Uuid,
    response: Value,
) -> AppResult<Registration> {
    let binding = session_binding(actor)?;
    let verifier = verifier(ports)?;
    let now = ports.clock.now();
    let challenge = ports
        .passkeys
        .take_challenge(challenge_id, ChallengeKind::Registration, &binding, now)
        .await?
        .filter(|c| c.user_id == actor.user_id)
        .ok_or_else(invalid_challenge)?;
    let credential = verifier.finish_registration(response, challenge.state)?;
    let passkey = PasskeyRecord {
        id: Uuid::now_v7(),
        user_id: actor.user_id,
        name: challenge.name.ok_or_else(invalid_challenge)?,
        credential,
        created_at: now,
        last_used_at: None,
    };
    // The repository serializes first-factor activation, so two registrations cannot
    // replace each other's recovery codes or each return a different active set.
    let codes = new_backup_codes(ports).await?;
    let created_recovery = ports.passkeys.register(&passkey, &codes.hashes).await?;
    notify(
        ports,
        actor,
        "Passkey added",
        "A passkey was added to your account.",
    )
    .await?;
    Ok(Registration {
        passkey,
        backup_codes: created_recovery.then(|| {
            codes
                .display
                .into_iter()
                .map(|code| BackupCodeView {
                    code: Some(code),
                    consumed: false,
                })
                .collect()
        }),
    })
}

pub async fn remove(
    ports: &Ports,
    actor: &Actor,
    id: Uuid,
    password: &str,
    code: &str,
) -> AppResult<()> {
    actor.require_user()?;
    if !ports
        .passkeys
        .list(actor.user_id)
        .await?
        .iter()
        .any(|key| key.id == id)
    {
        return Err(AppError::BadRequest {
            code: 50035,
            message: "Unknown passkey.".into(),
        });
    }
    let credentials = confirm_password(ports, actor.user_id, password).await?;
    verify_second_factor(ports, actor.user_id, &credentials, code, "code").await?;
    if !ports.passkeys.remove(actor.user_id, id).await? {
        return Err(AppError::BadRequest {
            code: 50035,
            message: "Unknown passkey.".into(),
        });
    }
    notify(
        ports,
        actor,
        "Passkey removed",
        "A passkey was removed from your account.",
    )
    .await
}

async fn notify(ports: &Ports, actor: &Actor, subject: &str, body: &str) -> AppResult<()> {
    let user = require_user(ports, actor.user_id).await?;
    if let Some(address) = &user.email {
        account_mail::notify(ports, address, subject, body).await;
    }
    Ok(())
}
