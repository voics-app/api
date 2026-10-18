use chrono::{DateTime, Duration, Utc};

use crate::snowflake::Snowflake;

/// Longest allowed `max_age`: 7 days, in seconds.
pub const MAX_INVITE_AGE_SECONDS: u32 = 604_800;
pub const MAX_INVITE_USES: u32 = 100;
pub const INVITE_CODE_LENGTH: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub code: String,
    pub guild_id: Snowflake,
    pub channel_id: Snowflake,
    pub inviter_id: Option<Snowflake>,
    /// Seconds; 0 means never expires.
    pub max_age: u32,
    /// 0 means unlimited.
    pub max_uses: u32,
    pub uses: u32,
    pub temporary: bool,
    pub created_at: DateTime<Utc>,
}

impl Invite {
    pub fn expires_at(&self) -> Option<DateTime<Utc>> {
        (self.max_age > 0).then(|| self.created_at + Duration::seconds(self.max_age as i64))
    }

    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at().is_some_and(|at| at <= now)
    }

    pub fn is_exhausted(&self) -> bool {
        self.max_uses > 0 && self.uses >= self.max_uses
    }

    pub fn is_usable(&self, now: DateTime<Utc>) -> bool {
        !self.is_expired(now) && !self.is_exhausted()
    }

    /// Whether accepting it once more uses it up.
    pub fn is_last_use(&self) -> bool {
        self.max_uses > 0 && self.uses + 1 >= self.max_uses
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invite(max_age: u32, max_uses: u32, uses: u32, age_seconds: i64) -> Invite {
        Invite {
            code: "abcdefgh".into(),
            guild_id: Snowflake::new(1),
            channel_id: Snowflake::new(2),
            inviter_id: None,
            max_age,
            max_uses,
            uses,
            temporary: false,
            created_at: Utc::now() - Duration::seconds(age_seconds),
        }
    }

    #[test]
    fn usability_table() {
        let now = Utc::now();
        let cases = [
            (
                "never expires, unlimited",
                invite(0, 0, 500, 1_000_000),
                true,
            ),
            ("within max age", invite(3600, 0, 0, 10), true),
            ("past max age", invite(3600, 0, 0, 3601), false),
            ("uses remaining", invite(0, 5, 4, 0), true),
            ("uses exhausted", invite(0, 5, 5, 0), false),
        ];
        for (name, inv, usable) in cases {
            assert_eq!(inv.is_usable(now), usable, "{name}");
        }
    }

    #[test]
    fn last_use_detection() {
        assert!(invite(0, 1, 0, 0).is_last_use());
        assert!(!invite(0, 2, 0, 0).is_last_use());
        assert!(!invite(0, 0, 10, 0).is_last_use());
    }
}
