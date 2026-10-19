//! Snowflake identifiers (pinned interface P5).
//!
//! Layout: 42 bits of milliseconds since [`DISCORDY_EPOCH_MS`], 10 bits of
//! worker id and 12 bits of sequence. Serialized as decimal strings.

use std::fmt;
use std::str::FromStr;

/// 2026-01-01T00:00:00Z in Unix milliseconds.
pub const DISCORDY_EPOCH_MS: u64 = 1_767_225_600_000;
/// Ten days of milliseconds: the width of one message bucket.
pub const BUCKET_WIDTH_MS: u64 = 864_000_000;

const WORKER_BITS: u32 = 10;
const SEQUENCE_BITS: u32 = 12;
const MAX_WORKER_ID: u16 = (1 << WORKER_BITS) - 1;
const MAX_SEQUENCE: u16 = (1 << SEQUENCE_BITS) - 1;
const TIMESTAMP_SHIFT: u32 = WORKER_BITS + SEQUENCE_BITS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Snowflake(u64);

impl Snowflake {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn value(self) -> u64 {
        self.0
    }

    /// Milliseconds since the Discordy epoch.
    pub const fn epoch_ms(self) -> u64 {
        self.0 >> TIMESTAMP_SHIFT
    }

    /// Unix timestamp in milliseconds.
    pub const fn unix_ms(self) -> u64 {
        self.epoch_ms() + DISCORDY_EPOCH_MS
    }

    pub const fn worker_id(self) -> u16 {
        ((self.0 >> SEQUENCE_BITS) & MAX_WORKER_ID as u64) as u16
    }

    pub const fn sequence(self) -> u16 {
        (self.0 & MAX_SEQUENCE as u64) as u16
    }

    /// Message bucket: `floor((snowflake >> 22) / 864_000_000)`.
    pub const fn bucket(self) -> u64 {
        self.epoch_ms() / BUCKET_WIDTH_MS
    }

    /// Database representation (Postgres `bigint`).
    pub const fn as_i64(self) -> i64 {
        self.0 as i64
    }

    pub const fn from_i64(raw: i64) -> Self {
        Self(raw as u64)
    }
}

impl fmt::Display for Snowflake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Snowflake {
    type Err = SnowflakeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u64>()
            .map(Snowflake)
            .map_err(|_| SnowflakeError::Malformed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnowflakeError {
    #[error("worker id must be between 0 and 1023")]
    WorkerOutOfRange,
    #[error("snowflake must be a decimal string")]
    Malformed,
}

/// Pure generator. Time is passed in by the caller so the domain does no IO.
///
/// IDs are strictly increasing: when the clock goes backwards or the sequence
/// is exhausted within one millisecond, the logical clock advances instead.
#[derive(Debug)]
pub struct SnowflakeGenerator {
    worker_id: u16,
    last_epoch_ms: u64,
    sequence: u16,
}

impl SnowflakeGenerator {
    pub fn new(worker_id: u16) -> Result<Self, SnowflakeError> {
        if worker_id > MAX_WORKER_ID {
            return Err(SnowflakeError::WorkerOutOfRange);
        }
        Ok(Self {
            worker_id,
            last_epoch_ms: 0,
            sequence: 0,
        })
    }

    pub fn next(&mut self, now_unix_ms: u64) -> Snowflake {
        let now = now_unix_ms.saturating_sub(DISCORDY_EPOCH_MS);
        if now > self.last_epoch_ms {
            self.last_epoch_ms = now;
            self.sequence = 0;
        } else if self.sequence == MAX_SEQUENCE {
            self.last_epoch_ms += 1;
            self.sequence = 0;
        } else {
            self.sequence += 1;
        }
        Snowflake(
            (self.last_epoch_ms << TIMESTAMP_SHIFT)
                | ((self.worker_id as u64) << SEQUENCE_BITS)
                | self.sequence as u64,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = DISCORDY_EPOCH_MS + 1_000;

    #[test]
    fn encodes_timestamp_worker_and_sequence() {
        let mut generator = SnowflakeGenerator::new(5).unwrap();
        let id = generator.next(T0);
        assert_eq!(id.epoch_ms(), 1_000);
        assert_eq!(id.unix_ms(), T0);
        assert_eq!(id.worker_id(), 5);
        assert_eq!(id.sequence(), 0);
        assert_eq!(id.value(), (1_000 << 22) | (5 << 12));
    }

    #[test]
    fn increments_sequence_within_same_millisecond() {
        let mut generator = SnowflakeGenerator::new(1).unwrap();
        let a = generator.next(T0);
        let b = generator.next(T0);
        assert_eq!(b.sequence(), 1);
        assert!(b > a);
    }

    #[test]
    fn resets_sequence_on_new_millisecond() {
        let mut generator = SnowflakeGenerator::new(1).unwrap();
        generator.next(T0);
        generator.next(T0);
        let c = generator.next(T0 + 1);
        assert_eq!(c.sequence(), 0);
        assert_eq!(c.epoch_ms(), 1_001);
    }

    #[test]
    fn stays_monotonic_when_clock_goes_backwards() {
        let mut generator = SnowflakeGenerator::new(1).unwrap();
        let a = generator.next(T0);
        let b = generator.next(T0 - 500);
        assert!(b > a);
    }

    #[test]
    fn advances_logical_clock_when_sequence_exhausted() {
        let mut generator = SnowflakeGenerator::new(1).unwrap();
        let mut last = generator.next(T0);
        for _ in 0..4096 {
            let next = generator.next(T0);
            assert!(next > last);
            last = next;
        }
        assert_eq!(last.epoch_ms(), 1_001);
    }

    #[test]
    fn rejects_out_of_range_worker() {
        assert_eq!(
            SnowflakeGenerator::new(1024).unwrap_err(),
            SnowflakeError::WorkerOutOfRange
        );
        assert!(SnowflakeGenerator::new(1023).is_ok());
    }

    #[test]
    fn computes_ten_day_buckets() {
        let cases = [
            (0u64, 0u64),
            (BUCKET_WIDTH_MS - 1, 0),
            (BUCKET_WIDTH_MS, 1),
            (BUCKET_WIDTH_MS * 7 + 5, 7),
        ];
        for (epoch_ms, bucket) in cases {
            assert_eq!(
                Snowflake::new(epoch_ms << 22).bucket(),
                bucket,
                "epoch_ms={epoch_ms}"
            );
        }
    }

    #[test]
    fn round_trips_decimal_strings() {
        let id = Snowflake::new(1_234_567_890_123_456_789);
        assert_eq!(id.to_string(), "1234567890123456789");
        assert_eq!("1234567890123456789".parse::<Snowflake>().unwrap(), id);
        assert_eq!(
            "abc".parse::<Snowflake>().unwrap_err(),
            SnowflakeError::Malformed
        );
    }

    #[test]
    fn round_trips_postgres_bigint() {
        let id = Snowflake::new(u64::MAX >> 1);
        assert_eq!(Snowflake::from_i64(id.as_i64()), id);
    }
}
