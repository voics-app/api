//! Two-factor authentication rules: backup codes and the authenticator URI.

/// Backup codes issued per enrolment or regeneration.
pub const BACKUP_CODE_COUNT: usize = 10;
/// Characters per backup code, without the separator.
pub const BACKUP_CODE_LENGTH: usize = 10;
/// Digits in a TOTP code.
pub const TOTP_DIGITS: usize = 6;
/// Shown in authenticator apps next to the account name.
pub const TOTP_ISSUER: &str = "Discordy";

/// A stored backup code: only its hash is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupCodeRecord {
    pub id: i64,
    pub hash: String,
    pub consumed: bool,
}

/// Canonical form of a backup code: lowercase, no separators or spaces.
/// `None` when the input cannot be a backup code.
pub fn normalize_backup_code(input: &str) -> Option<String> {
    let cleaned: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let valid =
        cleaned.len() == BACKUP_CODE_LENGTH && cleaned.chars().all(|c| c.is_ascii_alphanumeric());
    valid.then_some(cleaned)
}

/// `xxxxx-xxxxx`, how a backup code is displayed.
pub fn format_backup_code(canonical: &str) -> String {
    let (head, tail) = canonical.split_at(canonical.len() / 2);
    format!("{head}-{tail}")
}

/// Whether `input` has the shape of a TOTP code rather than a backup code.
pub fn looks_like_totp(input: &str) -> bool {
    let trimmed = input.trim();
    trimmed.len() == TOTP_DIGITS && trimmed.chars().all(|c| c.is_ascii_digit())
}

/// The `otpauth://` URI authenticator apps import from a QR code.
pub fn otpauth_uri(account: &str, secret: &str) -> String {
    format!(
        "otpauth://totp/{issuer}:{account}?secret={secret}&issuer={issuer}&algorithm=SHA1&digits={digits}&period=30",
        issuer = TOTP_ISSUER,
        account = percent_encode(account),
        digits = TOTP_DIGITS,
    )
}

fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte))
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_codes_normalize_regardless_of_case_and_separators() {
        assert_eq!(
            normalize_backup_code(" ABCDE-fghij "),
            Some("abcdefghij".to_owned())
        );
        assert_eq!(
            normalize_backup_code("abcdefghij"),
            Some("abcdefghij".to_owned())
        );
    }

    #[test]
    fn malformed_backup_codes_are_rejected() {
        assert_eq!(normalize_backup_code("short"), None);
        assert_eq!(normalize_backup_code("abcde-fghi!"), None);
        assert_eq!(normalize_backup_code("abcdefghijk"), None);
    }

    #[test]
    fn backup_codes_display_in_two_groups() {
        assert_eq!(format_backup_code("abcdefghij"), "abcde-fghij");
    }

    #[test]
    fn totp_codes_are_six_digits() {
        assert!(looks_like_totp("123456"));
        assert!(looks_like_totp(" 123456 "));
        assert!(!looks_like_totp("12345"));
        assert!(!looks_like_totp("abcdef"));
        assert!(!looks_like_totp("abcde-fghij"));
    }

    #[test]
    fn otpauth_uri_names_issuer_and_escapes_the_account() {
        let uri = otpauth_uri("alice+test@example.com", "JBSWY3DPEHPK3PXP");
        assert_eq!(
            uri,
            "otpauth://totp/Discordy:alice%2Btest%40example.com?secret=JBSWY3DPEHPK3PXP&issuer=Discordy&algorithm=SHA1&digits=6&period=30"
        );
    }
}
