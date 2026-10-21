//! Account lifecycle: emailed tokens, disabled and scheduled-for-deletion state.

use chrono::{DateTime, Utc};

use crate::snowflake::Snowflake;

/// What an emailed or ticketed single-use token authorises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenPurpose {
    VerifyEmail,
    ResetPassword,
    /// Proof that the password step of a login passed; awaits the second factor.
    MfaLogin,
}

impl TokenPurpose {
    pub const fn as_str(self) -> &'static str {
        match self {
            TokenPurpose::VerifyEmail => "verify_email",
            TokenPurpose::ResetPassword => "reset_password",
            TokenPurpose::MfaLogin => "mfa_login",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "verify_email" => Some(TokenPurpose::VerifyEmail),
            "reset_password" => Some(TokenPurpose::ResetPassword),
            "mfa_login" => Some(TokenPurpose::MfaLogin),
            _ => None,
        }
    }
}

/// A single-use token; only its hash is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountToken {
    pub token_hash: String,
    pub user_id: Snowflake,
    pub purpose: TokenPurpose,
    /// The address being verified: set for email changes, absent for the current one.
    pub email: Option<String>,
    pub expires_at: DateTime<Utc>,
}

/// Disable and deletion state; both clear on the next successful login.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AccountState {
    pub disabled_at: Option<DateTime<Utc>>,
    /// When a scheduled deletion becomes final.
    pub delete_after: Option<DateTime<Utc>>,
}

impl AccountState {
    pub fn is_inactive(&self) -> bool {
        self.disabled_at.is_some() || self.delete_after.is_some()
    }
}
