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
}
