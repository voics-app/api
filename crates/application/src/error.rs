use domain::application::InteractionError;
use domain::delivery_mode::DeliveryModeError;
use domain::device::KeyPackageError;
use domain::relationship::RelationshipError;
use domain::validation::{FieldError, ValidationErrors};

/// Things that can be unknown (Discord's 10xxx codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    Application,
    Ban,
    Channel,
    Command,
    Device,
    Emoji,
    SoundboardSound,
    Guild,
    GuildTemplate,
    Interaction,
    Invite,
    KeyPackage,
    Member,
    Message,
    Overwrite,
    Report,
    Role,
    Session,
    User,
    Webhook,
}

/// Why access was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denial {
    /// 50001: the resource exists but the caller cannot reach it.
    MissingAccess,
    /// 50013: the caller lacks a permission or hierarchy rank.
    MissingPermissions,
    /// The operation is only for user accounts, not bots.
    BotsNotAllowed,
    /// The caller is banned from the guild (40007).
    Banned,
    /// Cannot message or befriend this user (50007).
    CannotMessageUser,
    /// The account is suspended by a platform admin.
    Suspended,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("unauthorized")]
    Unauthorized,
    /// Login needs a second factor (Discord 60008 "MFA required"); `ticket`
    /// completes it through `POST /auth/mfa/totp`.
    #[error("two-factor code required")]
    MfaRequired {
        ticket: String,
        methods: Vec<&'static str>,
    },
    /// Too many attempts; retry after the given number of seconds.
    #[error("rate limited")]
    RateLimited { retry_after_secs: u64 },
    #[error("access denied: {0:?}")]
    Forbidden(Denial),
    #[error("unknown {0:?}")]
    NotFound(Resource),
    #[error("{0}")]
    Validation(ValidationErrors),
    #[error("{message}")]
    Conflict { code: u32, message: String },
    #[error("{message}")]
    BadRequest { code: u32, message: String },
    #[error("upstream unavailable: {0}")]
    Unavailable(String),
    #[error("internal error: {0}")]
    Internal(String),
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn invalid(field: &str, code: &'static str, message: impl Into<String>) -> Self {
        AppError::Validation(ValidationErrors::single(field, code, message))
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        AppError::Internal(error.to_string())
    }

    pub const fn missing_permissions() -> Self {
        AppError::Forbidden(Denial::MissingPermissions)
    }

    pub const fn missing_access() -> Self {
        AppError::Forbidden(Denial::MissingAccess)
    }
}

impl From<ValidationErrors> for AppError {
    fn from(errors: ValidationErrors) -> Self {
        AppError::Validation(errors)
    }
}

impl From<FieldError> for AppError {
    fn from(error: FieldError) -> Self {
        AppError::Validation(error.into())
    }
}

impl From<DeliveryModeError> for AppError {
    fn from(error: DeliveryModeError) -> Self {
        AppError::invalid("delivery_mode", "DELIVERY_MODE_INVALID", error.to_string())
    }
}

impl From<KeyPackageError> for AppError {
    fn from(error: KeyPackageError) -> Self {
        AppError::invalid("packages", "KEY_PACKAGES_INVALID", error.to_string())
    }
}

impl From<RelationshipError> for AppError {
    fn from(error: RelationshipError) -> Self {
        match error {
            RelationshipError::BlockedByTarget => AppError::Forbidden(Denial::CannotMessageUser),
            other => AppError::BadRequest {
                code: 80004,
                message: other.to_string(),
            },
        }
    }
}

impl From<InteractionError> for AppError {
    fn from(error: InteractionError) -> Self {
        match error {
            InteractionError::AlreadyAcknowledged => AppError::BadRequest {
                code: 40060,
                message: error.to_string(),
            },
            InteractionError::Expired => AppError::NotFound(Resource::Interaction),
            InteractionError::InvalidCallback | InteractionError::NotAcknowledged => {
                AppError::BadRequest {
                    code: 50035,
                    message: error.to_string(),
                }
            }
        }
    }
}
