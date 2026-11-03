//! The error envelope and Discord-style codes (P9).

use serde::Serialize;
use serde_json::Value;

pub const GENERAL_ERROR: u32 = 0;
pub const RATE_LIMITED: u32 = 429;
pub const UNKNOWN_APPLICATION: u32 = 10002;
pub const UNKNOWN_CHANNEL: u32 = 10003;
pub const UNKNOWN_GUILD: u32 = 10004;
pub const UNKNOWN_INVITE: u32 = 10006;
pub const UNKNOWN_MEMBER: u32 = 10007;
pub const UNKNOWN_MESSAGE: u32 = 10008;
pub const UNKNOWN_ROLE: u32 = 10011;
pub const UNKNOWN_USER: u32 = 10013;
pub const UNKNOWN_EMOJI: u32 = 10014;
pub const UNKNOWN_WEBHOOK: u32 = 10015;
pub const UNKNOWN_BAN: u32 = 10026;
pub const UNKNOWN_GUILD_TEMPLATE: u32 = 10057;
pub const UNKNOWN_INTERACTION: u32 = 10062;
pub const UNKNOWN_APPLICATION_COMMAND: u32 = 10063;
pub const UNAUTHORIZED: u32 = 40001;
pub const VERIFICATION_REQUIRED: u32 = 40002;
pub const BANNED: u32 = 40007;
pub const MFA_REQUIRED: u32 = 60008;
pub const MISSING_ACCESS: u32 = 50001;
pub const MISSING_PERMISSIONS: u32 = 50013;
pub const CANNOT_MESSAGE_USER: u32 = 50007;
pub const INVALID_FORM_BODY: u32 = 50035;
pub const DELIVERY_MODE_CONFLICT: u32 = 50160;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ErrorBody {
    pub code: u32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Value>,
}

impl ErrorBody {
    pub fn new(code: u32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            errors: None,
        }
    }
}
