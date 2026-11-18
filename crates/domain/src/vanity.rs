use chrono::{DateTime, Utc};

use crate::validation::FieldError;
use crate::Snowflake;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VanityInvite {
    pub guild_id: Snowflake,
    pub code: String,
    pub uses: u32,
    pub created_at: DateTime<Utc>,
}

pub fn normalize_code(code: &str) -> Result<String, FieldError> {
    if !(2..=32).contains(&code.len())
        || !code.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err(FieldError {
            field: "code".into(),
            code: "VANITY_CODE_INVALID",
            message: "Use 2 to 32 letters, digits or hyphens.".into(),
        });
    }
    Ok(code.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_normalize_without_accepting_paths_whitespace_or_unicode() {
        assert_eq!(normalize_code("Study-Club").unwrap(), "study-club");
        for code in ["", "a", " two ", "two/three", "foo?bar", "café", "😀"] {
            assert!(normalize_code(code).is_err(), "{code}");
        }
        assert!(normalize_code(&"a".repeat(32)).is_ok());
        assert!(normalize_code(&"a".repeat(33)).is_err());
    }
}
