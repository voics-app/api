use chrono::{DateTime, Utc};

use crate::snowflake::Snowflake;

pub const IDENTITY_KEY_LENGTH: usize = 32;
pub const MAX_KEY_PACKAGES_PER_UPLOAD: usize = 100;
pub const MAX_KEY_PACKAGE_BYTES: usize = 16 * 1024;
pub const MAX_DEVICES_PER_USER: usize = 20;

/// A client installation with its Ed25519 identity key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: Snowflake,
    pub user_id: Snowflake,
    pub name: String,
    pub identity_key: [u8; IDENTITY_KEY_LENGTH],
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyPackageError {
    #[error("between 1 and 100 key packages must be uploaded at once")]
    BadCount,
    #[error("key packages must be between 1 byte and 16 KiB")]
    BadSize,
}

pub fn validate_key_packages(packages: &[Vec<u8>]) -> Result<(), KeyPackageError> {
    if packages.is_empty() || packages.len() > MAX_KEY_PACKAGES_PER_UPLOAD {
        return Err(KeyPackageError::BadCount);
    }
    if packages
        .iter()
        .any(|p| p.is_empty() || p.len() > MAX_KEY_PACKAGE_BYTES)
    {
        return Err(KeyPackageError::BadSize);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_package_validation_table() {
        let cases = [
            (vec![], Err(KeyPackageError::BadCount)),
            (vec![vec![1u8; 10]], Ok(())),
            (vec![vec![1u8; 10]; 101], Err(KeyPackageError::BadCount)),
            (vec![vec![]], Err(KeyPackageError::BadSize)),
            (
                vec![vec![0u8; MAX_KEY_PACKAGE_BYTES + 1]],
                Err(KeyPackageError::BadSize),
            ),
        ];
        for (packages, expected) in cases {
            assert_eq!(
                validate_key_packages(&packages),
                expected,
                "{} packages",
                packages.len()
            );
        }
    }
}
