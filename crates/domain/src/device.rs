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

/// The device to evict once the limit is reached: the oldest one that no
/// live session is bound to. `None` when every device is still in use.
pub fn eviction_candidate(devices: &[Device], live_device_ids: &[Snowflake]) -> Option<Snowflake> {
    devices
        .iter()
        .filter(|device| !live_device_ids.contains(&device.id))
        .min_by_key(|device| (device.created_at, device.id))
        .map(|device| device.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: u64, created_at: i64) -> Device {
        Device {
            id: Snowflake::new(id),
            user_id: Snowflake::new(1),
            name: format!("device {id}"),
            identity_key: [0; IDENTITY_KEY_LENGTH],
            created_at: DateTime::from_timestamp(created_at, 0).expect("timestamp"),
        }
    }

    #[test]
    fn evicts_the_oldest_device_without_a_live_session() {
        let devices = [device(10, 300), device(11, 100), device(12, 200)];
        let live = [Snowflake::new(11)];
        assert_eq!(
            eviction_candidate(&devices, &live),
            Some(Snowflake::new(12))
        );
    }

    #[test]
    fn evicts_nothing_when_every_device_has_a_live_session() {
        let devices = [device(10, 300), device(11, 100)];
        let live = [Snowflake::new(10), Snowflake::new(11)];
        assert_eq!(eviction_candidate(&devices, &live), None);
    }

    #[test]
    fn ties_on_creation_time_break_by_id() {
        let devices = [device(12, 100), device(10, 100)];
        assert_eq!(eviction_candidate(&devices, &[]), Some(Snowflake::new(10)));
    }

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
