//! Snowflake ids from the domain generator, stamped with the wall clock.

use std::sync::Mutex;

use chrono::Utc;

use application::ports::IdGenerator;
use domain::snowflake::{SnowflakeError, SnowflakeGenerator};
use domain::Snowflake;

pub struct SnowflakeIds {
    generator: Mutex<SnowflakeGenerator>,
}

impl SnowflakeIds {
    /// `worker_id` comes from `SNOWFLAKE_WORKER_ID` (P2, range per P5).
    pub fn new(worker_id: u16) -> Result<Self, SnowflakeError> {
        Ok(Self {
            generator: Mutex::new(SnowflakeGenerator::new(worker_id)?),
        })
    }
}

impl IdGenerator for SnowflakeIds {
    fn next_id(&self) -> Snowflake {
        let now_ms = u64::try_from(Utc::now().timestamp_millis()).unwrap_or_default();
        let mut generator = self
            .generator
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        generator.next(now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_strictly_increasing() {
        let ids = SnowflakeIds::new(1).unwrap();
        let first = ids.next_id();
        let second = ids.next_id();
        assert!(second.value() > first.value());
    }
}
