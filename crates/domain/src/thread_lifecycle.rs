//! Clock-driven thread archival decisions, independent of storage and workers.

use chrono::{DateTime, Duration, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadLifecycle {
    pub archived: bool,
    pub locked: bool,
    pub duration_minutes: u32,
    pub last_activity_at: DateTime<Utc>,
    pub archive_timestamp: DateTime<Utc>,
    pub automatically_archived: bool,
}

impl ThreadLifecycle {
    pub fn archive_if_due(&mut self, now: DateTime<Utc>) -> bool {
        if self.archived
            || now < self.last_activity_at + Duration::minutes(i64::from(self.duration_minutes))
        {
            return false;
        }
        self.archived = true;
        self.automatically_archived = true;
        self.archive_timestamp = now;
        true
    }

    /// Records a committed message; older duplicates cannot move activity backwards.
    /// A late message only reverses automatic archival while it is still recent.
    pub fn record_message(&mut self, occurred_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
        if occurred_at <= self.last_activity_at {
            return false;
        }
        self.last_activity_at = occurred_at;
        let recent = now < occurred_at + Duration::minutes(i64::from(self.duration_minutes));
        let after_manual_archive = occurred_at > self.archive_timestamp;
        if self.archived
            && !self.locked
            && recent
            && (self.automatically_archived || after_manual_archive)
        {
            self.archived = false;
            self.automatically_archived = false;
            self.archive_timestamp = now;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(minutes: i64) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap() + Duration::minutes(minutes)
    }

    fn state() -> ThreadLifecycle {
        ThreadLifecycle {
            archived: false,
            locked: false,
            duration_minutes: 60,
            last_activity_at: at(0),
            archive_timestamp: at(0),
            automatically_archived: false,
        }
    }

    #[test]
    fn archives_only_at_or_after_the_inactivity_deadline() {
        let mut thread = state();
        assert!(!thread.archive_if_due(at(59)));
        assert!(thread.archive_if_due(at(60)));
        assert!(thread.archived && thread.automatically_archived);
        assert_eq!(thread.archive_timestamp, at(60));
        assert!(!thread.archive_if_due(at(61)));
    }

    #[test]
    fn recent_message_reopens_an_auto_archived_thread_even_when_delivery_is_delayed() {
        let mut thread = state();
        assert!(thread.archive_if_due(at(60)));
        assert!(thread.record_message(at(59), at(61)));
        assert!(!thread.archived);
        assert_eq!(thread.last_activity_at, at(59));
    }

    #[test]
    fn old_or_duplicate_messages_do_not_resurrect_a_thread() {
        let mut thread = state();
        assert!(thread.archive_if_due(at(60)));
        assert!(!thread.record_message(at(0), at(61)));
        assert!(!thread.record_message(at(1), at(120)));
        assert!(thread.archived);
    }

    #[test]
    fn manual_archive_and_lock_ignore_earlier_messages() {
        let mut thread = state();
        thread.archived = true;
        thread.archive_timestamp = at(10);
        assert!(!thread.record_message(at(9), at(11)));
        assert!(thread.archived);
        thread.locked = true;
        assert!(!thread.record_message(at(11), at(12)));
        assert!(thread.archived);
        thread.locked = false;
        assert!(thread.record_message(at(12), at(13)));
        assert!(!thread.archived);
    }
}
