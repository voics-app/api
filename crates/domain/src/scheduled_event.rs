//! Rules and values for scheduled guild events.

use crate::Snowflake;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    Stage { channel_id: Snowflake },
    Voice { channel_id: Snowflake },
    External { location: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventStatus {
    Scheduled,
    Active,
    Completed,
    Canceled,
}

impl EventStatus {
    pub const fn code(self) -> i16 {
        match self {
            Self::Scheduled => 1,
            Self::Active => 2,
            Self::Completed => 3,
            Self::Canceled => 4,
        }
    }

    pub const fn from_code(code: i16) -> Option<Self> {
        match code {
            1 => Some(Self::Scheduled),
            2 => Some(Self::Active),
            3 => Some(Self::Completed),
            4 => Some(Self::Canceled),
            _ => None,
        }
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        self == next
            || matches!(
                (self, next),
                (Self::Scheduled, Self::Active | Self::Canceled) | (Self::Active, Self::Completed)
            )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledEventDetails {
    pub name: String,
    pub description: Option<String>,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    pub kind: EventKind,
    pub status: EventStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledEvent {
    pub id: Snowflake,
    pub guild_id: Snowflake,
    pub creator_id: Option<Snowflake>,
    pub details: ScheduledEventDetails,
    pub image: Option<String>,
    pub user_count: u64,
    /// Internal optimistic-concurrency and reminder revision.
    pub revision: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventValidationError {
    Name,
    Description,
    End,
    Location,
}

impl ScheduledEventDetails {
    pub fn validate(&self) -> Result<(), EventValidationError> {
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 100 {
            return Err(EventValidationError::Name);
        }
        if self
            .description
            .as_ref()
            .is_some_and(|d| d.chars().count() > 1000)
        {
            return Err(EventValidationError::Description);
        }
        if self.end.is_some_and(|end| end <= self.start)
            || (matches!(self.kind, EventKind::External { .. }) && self.end.is_none())
        {
            return Err(EventValidationError::End);
        }
        if let EventKind::External { location } = &self.kind {
            let location = location.trim();
            if location.is_empty() || location.chars().count() > 100 {
                return Err(EventValidationError::Location);
            }
        }
        Ok(())
    }
}
