//! Input rules shared by every use case. Violations surface as Discord's
//! "invalid form body" (50035) with per-field errors.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    pub field: String,
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ValidationErrors(Vec<FieldError>);

impl ValidationErrors {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn single(field: &str, code: &'static str, message: impl Into<String>) -> Self {
        let mut errors = Self::new();
        errors.add(field, code, message);
        errors
    }

    pub fn add(&mut self, field: &str, code: &'static str, message: impl Into<String>) {
        self.0.push(FieldError {
            field: field.to_owned(),
            code,
            message: message.into(),
        });
    }

    /// Records the error of a check, if any.
    pub fn check(&mut self, result: Result<(), FieldError>) {
        if let Err(error) = result {
            self.0.push(error);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn errors(&self) -> &[FieldError] {
        &self.0
    }

    pub fn into_result(self) -> Result<(), ValidationErrors> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(self)
        }
    }
}

impl From<FieldError> for ValidationErrors {
    fn from(error: FieldError) -> Self {
        Self(vec![error])
    }
}

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fields: Vec<&str> = self.0.iter().map(|e| e.field.as_str()).collect();
        write!(f, "invalid fields: {}", fields.join(", "))
    }
}

impl std::error::Error for ValidationErrors {}

fn error(field: &str, code: &'static str, message: impl Into<String>) -> FieldError {
    FieldError {
        field: field.to_owned(),
        code,
        message: message.into(),
    }
}

fn length_between(field: &str, value: &str, min: usize, max: usize) -> Result<(), FieldError> {
    let length = value.chars().count();
    if length < min || length > max {
        return Err(error(
            field,
            "BASE_TYPE_BAD_LENGTH",
            format!("Must be between {min} and {max} in length."),
        ));
    }
    Ok(())
}

/// Lowercase letters, digits, `_` and `.`; 2–32 characters; no `..`.
pub fn username(value: &str) -> Result<(), FieldError> {
    length_between("username", value, 2, 32)?;
    let allowed = value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.');
    if !allowed || value.contains("..") {
        return Err(error(
            "username",
            "USERNAME_INVALID_CHARACTERS",
            "Username can only contain lowercase letters, numbers, underscores and periods.",
        ));
    }
    Ok(())
}

pub fn global_name(value: &str) -> Result<(), FieldError> {
    length_between("global_name", value.trim(), 1, 32)
}

pub fn email(value: &str) -> Result<(), FieldError> {
    let valid = value.len() <= 254
        && value.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        })
        && !value.contains(char::is_whitespace);
    if !valid {
        return Err(error(
            "email",
            "EMAIL_TYPE_INVALID_EMAIL",
            "Not a well formed email address.",
        ));
    }
    Ok(())
}

pub fn password(value: &str) -> Result<(), FieldError> {
    let length = value.chars().count();
    if !(8..=128).contains(&length) {
        return Err(error(
            "password",
            "PASSWORD_BAD_LENGTH",
            "Must be between 8 and 128 in length.",
        ));
    }
    Ok(())
}

pub fn guild_name(value: &str) -> Result<(), FieldError> {
    length_between("name", value.trim(), 2, 100)
}

pub fn template_name(value: &str) -> Result<(), FieldError> {
    length_between("name", value.trim(), 1, 100)
}

pub fn template_description(value: &str) -> Result<(), FieldError> {
    length_between("description", value, 0, 120)
}

pub fn channel_name(value: &str) -> Result<(), FieldError> {
    length_between("name", value.trim(), 1, 100)
}

pub fn channel_topic(value: &str) -> Result<(), FieldError> {
    length_between("topic", value, 0, 1024)
}

pub const ROLE_NAME_MAX: usize = 100;

pub fn role_name(value: &str) -> Result<(), FieldError> {
    length_between("name", value.trim(), 1, ROLE_NAME_MAX)
}

pub fn nickname(value: &str) -> Result<(), FieldError> {
    length_between("nick", value.trim(), 1, 32)
}

pub fn application_name(value: &str) -> Result<(), FieldError> {
    length_between("name", value.trim(), 2, 32)
}

pub fn webhook_name(value: &str) -> Result<(), FieldError> {
    length_between("name", value.trim(), 1, 80)
}

pub fn audit_reason(value: &str) -> Result<(), FieldError> {
    length_between("reason", value, 0, 512)
}

/// Letters, digits and `_`; 2–32 characters.
pub fn emoji_name(value: &str) -> Result<(), FieldError> {
    length_between("name", value, 2, 32)?;
    if !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(error(
            "name",
            "EMOJI_NAME_INVALID",
            "Emoji names may only contain letters, numbers and _.",
        ));
    }
    Ok(())
}

/// Text-like channel names are lowercase with dashes instead of spaces.
pub fn normalize_text_channel_name(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_table() {
        let cases = [
            ("alice", true),
            ("a.b_c9", true),
            ("ab", true),
            ("a", false),
            (&"x".repeat(33) as &str, false),
            ("Alice", false),
            ("al ice", false),
            ("a..b", false),
            ("al!ce", false),
        ];
        for (value, valid) in cases {
            assert_eq!(username(value).is_ok(), valid, "{value}");
        }
    }

    #[test]
    fn email_table() {
        let cases = [
            ("a@example.com", true),
            ("a.b+c@sub.example.org", true),
            ("no-at.example.com", false),
            ("@example.com", false),
            ("a@localhost", false),
            ("a@.com", false),
            ("a b@example.com", false),
        ];
        for (value, valid) in cases {
            assert_eq!(email(value).is_ok(), valid, "{value}");
        }
    }

    #[test]
    fn password_length_bounds() {
        assert!(password("1234567").is_err());
        assert!(password("12345678").is_ok());
        assert!(password(&"x".repeat(129)).is_err());
    }

    #[test]
    fn emoji_name_table() {
        for (value, valid) in [
            ("party_parrot", true),
            ("x", false),
            ("bad-name", false),
            ("ok2", true),
        ] {
            assert_eq!(emoji_name(value).is_ok(), valid, "{value}");
        }
    }

    #[test]
    fn length_rules() {
        assert!(guild_name("a").is_err());
        assert!(guild_name("ab").is_ok());
        assert!(channel_name("").is_err());
        assert!(role_name(&"r".repeat(101)).is_err());
        assert!(nickname("  ").is_err());
    }

    #[test]
    fn normalizes_text_channel_names() {
        assert_eq!(
            normalize_text_channel_name("  General   Chat "),
            "general-chat"
        );
    }

    #[test]
    fn collects_multiple_errors() {
        let mut errors = ValidationErrors::new();
        errors.check(username("A"));
        errors.check(email("nope"));
        errors.check(password("long enough"));
        let result = errors.into_result().unwrap_err();
        let fields: Vec<_> = result.errors().iter().map(|e| e.field.as_str()).collect();
        assert_eq!(fields, ["username", "email"]);
    }
}
