//! Account emails: verification, password reset and security notices.

use chrono::{DateTime, Duration, Utc};

use domain::account::{AccountToken, TokenPurpose};
use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;
use crate::ports::Mail;
use crate::support::throttle::{self, MAIL_PER_RECIPIENT, MAIL_WINDOW};

/// Random bytes in an emailed token.
const TOKEN_BYTES: usize = 24;
const VERIFY_TTL_HOURS: i64 = 24;
const RESET_TTL_HOURS: i64 = 1;
/// How long a password-verified login waits for its second factor.
pub const MFA_TICKET_MINUTES: i64 = 5;

/// Sends one mail, throttled per recipient.
pub async fn deliver(ports: &Ports, to: &str, subject: &str, body: String) -> AppResult<()> {
    throttle::hit(
        ports,
        &format!("mail:{}", to.to_lowercase()),
        MAIL_PER_RECIPIENT,
        MAIL_WINDOW,
    )
    .await?;
    ports
        .mailer
        .send(&Mail {
            to: to.to_owned(),
            subject: subject.to_owned(),
            body,
        })
        .await
}

async fn issue(
    ports: &Ports,
    user_id: Snowflake,
    purpose: TokenPurpose,
    email: Option<String>,
    expires_at: DateTime<Utc>,
) -> AppResult<String> {
    let token = ports.secrets.token(TOKEN_BYTES);
    ports
        .account_tokens
        .create(&AccountToken {
            token_hash: ports.secrets.hash(&token),
            user_id,
            purpose,
            email,
            expires_at,
        })
        .await?;
    Ok(token)
}

/// The configured product name (`BRAND_NAME`).
pub fn brand(ports: &Ports) -> &str {
    &ports.settings.brand_name
}

fn link(ports: &Ports, action: &str, token: &str) -> String {
    format!("{}{action}?token={token}", ports.settings.app_link_base)
}

/// Emails a verification token for `address` (the user's own or a new one).
/// A newer token voids the older ones.
pub async fn send_verification(
    ports: &Ports,
    user_id: Snowflake,
    username: &str,
    address: &str,
    is_change: bool,
) -> AppResult<()> {
    let now = ports.clock.now();
    ports
        .account_tokens
        .revoke_for_user(user_id, TokenPurpose::VerifyEmail, now)
        .await?;
    let pending = is_change.then(|| address.to_owned());
    let expires_at = now + Duration::hours(VERIFY_TTL_HOURS);
    let token = issue(
        ports,
        user_id,
        TokenPurpose::VerifyEmail,
        pending,
        expires_at,
    )
    .await?;
    let brand = brand(ports);
    let body = format!(
        "Hi {username},\n\nConfirm this email address for your {brand} account:\n{}\n\n\
         If the link does not open, paste this code into {brand}: {token}\n\n\
         It expires in {VERIFY_TTL_HOURS} hours. If you did not ask for this, ignore this email.\n",
        link(ports, "verify-email", &token),
    );
    deliver(ports, address, &format!("Verify your {brand} email"), body).await
}

pub async fn send_password_reset(
    ports: &Ports,
    user_id: Snowflake,
    username: &str,
    address: &str,
) -> AppResult<()> {
    let now = ports.clock.now();
    ports
        .account_tokens
        .revoke_for_user(user_id, TokenPurpose::ResetPassword, now)
        .await?;
    let expires_at = now + Duration::hours(RESET_TTL_HOURS);
    let token = issue(
        ports,
        user_id,
        TokenPurpose::ResetPassword,
        None,
        expires_at,
    )
    .await?;
    let brand = brand(ports);
    let body = format!(
        "Hi {username},\n\nSomeone asked to reset the password of your {brand} account:\n{}\n\n\
         If the link does not open, paste this code into {brand}: {token}\n\n\
         It expires in {RESET_TTL_HOURS} hour and signs you out everywhere. \
         If this was not you, ignore this email; your password is unchanged.\n",
        link(ports, "reset-password", &token),
    );
    deliver(
        ports,
        address,
        &format!("Reset your {brand} password"),
        body,
    )
    .await
}

/// Single-use proof of a passed password step, valid for [`MFA_TICKET_MINUTES`].
pub async fn issue_mfa_ticket(ports: &Ports, user_id: Snowflake) -> AppResult<String> {
    let expires_at = ports.clock.now() + Duration::minutes(MFA_TICKET_MINUTES);
    issue(ports, user_id, TokenPurpose::MfaLogin, None, expires_at).await
}

/// Tells `address` about a security-relevant change; failures are logged, never raised.
pub async fn notify(ports: &Ports, address: &str, subject: &str, detail: &str) {
    let body = format!("{detail}\n\nIf this was not you, reset your password right away.\n");
    if let Err(error) = deliver(ports, address, subject, body).await {
        tracing::warn!(%error, "security notice not delivered");
    }
}
