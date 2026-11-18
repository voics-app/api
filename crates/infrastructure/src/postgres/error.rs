use application::AppError;

/// Maps a driver error onto an application error. Unique violations on
/// user-chosen names become form errors; the rest stay opaque.
pub fn db(error: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(database) = &error {
        match (database.code().as_deref(), database.constraint()) {
            (Some("23505"), Some("guild_vanity_urls_code_key")) => {
                return super::vanity::occupied_code();
            }
            (Some("23505"), Some("users_email_key")) => {
                return AppError::invalid(
                    "email",
                    "EMAIL_ALREADY_REGISTERED",
                    "Email is already registered.",
                )
            }
            (Some("23505"), Some("users_username_key")) => {
                return AppError::invalid(
                    "username",
                    "USERNAME_ALREADY_TAKEN",
                    "Username is unavailable.",
                )
            }
            (Some("23505"), _) => {
                return AppError::Conflict {
                    code: 0,
                    message: "The resource already exists.".into(),
                }
            }
            (Some("23503"), _) => {
                return AppError::Conflict {
                    code: 0,
                    message: "A referenced resource no longer exists.".into(),
                }
            }
            _ => {}
        }
    }
    tracing::error!(%error, "database error");
    AppError::internal(error)
}
