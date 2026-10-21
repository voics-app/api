//! RFC 6238 time-based one-time passwords (SHA-1, 6 digits, 30 s step).

use chrono::{DateTime, Utc};
use totp_rs::{Algorithm, Secret, TOTP};

use application::ports::TotpVerifier;

const DIGITS: usize = 6;
const SKEW: u8 = 1;
const STEP_SECONDS: u64 = 30;

#[derive(Debug, Default, Clone, Copy)]
pub struct Rfc6238;

impl TotpVerifier for Rfc6238 {
    fn verify(&self, secret: &str, code: &str, now: DateTime<Utc>) -> bool {
        let Ok(bytes) = Secret::Encoded(secret.to_owned()).to_bytes() else {
            return false;
        };
        let Ok(totp) = TOTP::new(Algorithm::SHA1, DIGITS, SKEW, STEP_SECONDS, bytes) else {
            return false;
        };
        let Ok(seconds) = u64::try_from(now.timestamp()) else {
            return false;
        };
        totp.check(code, seconds)
    }

    fn generate_secret(&self) -> String {
        Secret::generate_secret().to_encoded().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn codes_generated_for_a_secret_verify_within_the_window() {
        let verifier = Rfc6238;
        let secret = verifier.generate_secret();
        let bytes = Secret::Encoded(secret.clone()).to_bytes().unwrap();
        let totp = TOTP::new(Algorithm::SHA1, DIGITS, SKEW, STEP_SECONDS, bytes).unwrap();
        let at = Utc.with_ymd_and_hms(2026, 10, 18, 12, 0, 0).unwrap();
        let code = totp.generate(at.timestamp() as u64);
        assert!(verifier.verify(&secret, &code, at));
        let ten_minutes_later = at + chrono::Duration::minutes(10);
        let stale = totp.generate(ten_minutes_later.timestamp() as u64);
        assert!(!verifier.verify(&secret, &stale, at));
    }

    #[test]
    fn malformed_secrets_never_verify() {
        assert!(!Rfc6238.verify("not base32!", "123456", Utc::now()));
    }

    /// RFC 6238 appendix B (SHA-1, secret "12345678901234567890"); the RFC lists
    /// 8-digit codes, of which the 6-digit code is the last six digits.
    const RFC_SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
    const RFC_VECTORS: [(i64, &str); 6] = [
        (59, "287082"),
        (1_111_111_109, "081804"),
        (1_111_111_111, "050471"),
        (1_234_567_890, "005924"),
        (2_000_000_000, "279037"),
        (20_000_000_000, "353130"),
    ];

    #[test]
    fn rfc_6238_vectors_verify_at_their_timestamps() {
        for (seconds, code) in RFC_VECTORS {
            let at = Utc.timestamp_opt(seconds, 0).unwrap();
            assert!(Rfc6238.verify(RFC_SECRET, code, at), "T={seconds}");
        }
    }

    #[test]
    fn one_step_of_skew_is_accepted_and_two_are_not() {
        let code = "081804";
        let at = |seconds: i64| Utc.timestamp_opt(seconds, 0).unwrap();
        let base = 1_111_111_109;
        assert!(Rfc6238.verify(RFC_SECRET, code, at(base + 30)));
        assert!(Rfc6238.verify(RFC_SECRET, code, at(base - 30)));
        assert!(!Rfc6238.verify(RFC_SECRET, code, at(base + 90)));
        assert!(!Rfc6238.verify(RFC_SECRET, "000000", at(base)));
    }
}
