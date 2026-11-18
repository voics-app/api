//! A password-issued ticket and a verified passkey jointly authorize one login.

use chrono::Duration;
use domain::account::{AccountToken, TokenPurpose};
use domain::session::ClientInfo;
use serde_json::Value;
use uuid::Uuid;

use crate::context::Ports;
use crate::error::{AppError, AppResult, Denial};
use crate::ports::{ChallengeKind, PasskeyChallenge, CHALLENGE_LIFETIME_SECONDS};
use crate::support::throttle::{self, MFA_ATTEMPTS, MFA_WINDOW};
use crate::support::views::require_user;
use crate::use_cases::mfa::passkeys::{invalid_challenge, verifier, Challenge};

use super::{login, login_mfa::invalid_ticket, tokens::AuthTokens};

async fn ticket(ports: &Ports, raw: &str) -> AppResult<AccountToken> {
    let ticket = ports
        .account_tokens
        .find_active(
            &ports.secrets.hash(raw),
            TokenPurpose::MfaLogin,
            ports.clock.now(),
        )
        .await?
        .ok_or_else(invalid_ticket)?;
    let user = require_user(ports, ticket.user_id).await?;
    if user.is_suspended(ports.clock.now()) {
        return Err(AppError::Forbidden(Denial::Suspended));
    }
    Ok(ticket)
}

pub async fn start(ports: &Ports, raw_ticket: &str) -> AppResult<Challenge> {
    let verifier = verifier(ports)?;
    let ticket = ticket(ports, raw_ticket).await?;
    throttle::hit(
        ports,
        &format!("passkey-start:{}", ticket.user_id),
        MFA_ATTEMPTS,
        MFA_WINDOW,
    )
    .await?;
    let keys = ports.passkeys.list(ticket.user_id).await?;
    if keys.is_empty() {
        return Err(invalid_ticket());
    }
    let ceremony = verifier.start_authentication(&keys)?;
    let now = ports.clock.now();
    let challenge = PasskeyChallenge {
        id: Uuid::now_v7(),
        user_id: ticket.user_id,
        kind: ChallengeKind::Authentication,
        binding: ticket.token_hash,
        name: None,
        state: ceremony.state,
        expires_at: ticket
            .expires_at
            .min(now + Duration::seconds(CHALLENGE_LIFETIME_SECONDS)),
    };
    ports.passkeys.create_challenge(&challenge, now).await?;
    Ok(Challenge {
        id: challenge.id,
        public_key: ceremony.public_key,
    })
}

pub async fn finish(
    ports: &Ports,
    raw_ticket: &str,
    challenge_id: Uuid,
    response: Value,
    client: ClientInfo,
) -> AppResult<AuthTokens> {
    let verifier = verifier(ports)?;
    let ticket = ticket(ports, raw_ticket).await?;
    throttle::hit(
        ports,
        &format!("mfa:{}", ticket.user_id),
        MFA_ATTEMPTS,
        MFA_WINDOW,
    )
    .await?;
    let now = ports.clock.now();
    let challenge = ports
        .passkeys
        .take_challenge(
            challenge_id,
            ChallengeKind::Authentication,
            &ticket.token_hash,
            now,
        )
        .await?
        .filter(|c| c.user_id == ticket.user_id)
        .ok_or_else(invalid_challenge)?;
    let keys = ports.passkeys.list(ticket.user_id).await?;
    let verified = verifier.finish_authentication(response, challenge.state, &keys)?;
    if !ports
        .passkeys
        .authenticate(ticket.user_id, &verified, &ticket.token_hash, now)
        .await?
    {
        return Err(invalid_ticket());
    }
    let user = require_user(ports, ticket.user_id).await?;
    login::complete(ports, user, None, client).await
}
