//! Reports of messages and users (api#126): categories, notes and review states.

use chrono::{DateTime, Utc};

use crate::delivery_mode::DeliveryMode;
use crate::validation::FieldError;
use crate::Snowflake;

pub const MAX_NOTE_CHARS: usize = 500;
/// Most characters of a reporter-submitted message copy; matches the message length limit.
pub const MAX_SUBMITTED_CONTENT_CHARS: usize = 4000;

fn invalid(field: &str, code: &'static str, message: &str) -> FieldError {
    FieldError {
        field: field.to_owned(),
        code,
        message: message.to_owned(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportCategory {
    Spam,
    Harassment,
    HateSpeech,
    Violence,
    SexualContent,
    SelfHarm,
    Illegal,
    Impersonation,
    Other,
}

impl ReportCategory {
    pub const ALL: [ReportCategory; 9] = [
        ReportCategory::Spam,
        ReportCategory::Harassment,
        ReportCategory::HateSpeech,
        ReportCategory::Violence,
        ReportCategory::SexualContent,
        ReportCategory::SelfHarm,
        ReportCategory::Illegal,
        ReportCategory::Impersonation,
        ReportCategory::Other,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            ReportCategory::Spam => "spam",
            ReportCategory::Harassment => "harassment",
            ReportCategory::HateSpeech => "hate_speech",
            ReportCategory::Violence => "violence",
            ReportCategory::SexualContent => "sexual_content",
            ReportCategory::SelfHarm => "self_harm",
            ReportCategory::Illegal => "illegal",
            ReportCategory::Impersonation => "impersonation",
            ReportCategory::Other => "other",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, FieldError> {
        Self::ALL
            .into_iter()
            .find(|category| category.as_str() == raw)
            .ok_or_else(|| {
                invalid(
                    "category",
                    "REPORT_CATEGORY_INVALID",
                    "Must be one of the report categories.",
                )
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    Message,
    User,
    Guild,
    Channel,
}

impl ReportKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            ReportKind::Message => "message",
            ReportKind::User => "user",
            ReportKind::Guild => "guild",
            ReportKind::Channel => "channel",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "message" => Some(ReportKind::Message),
            "user" => Some(ReportKind::User),
            "guild" => Some(ReportKind::Guild),
            "channel" => Some(ReportKind::Channel),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportStatus {
    Open,
    Resolved,
    Dismissed,
}

impl ReportStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            ReportStatus::Open => "open",
            ReportStatus::Resolved => "resolved",
            ReportStatus::Dismissed => "dismissed",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "open" => Some(ReportStatus::Open),
            "resolved" => Some(ReportStatus::Resolved),
            "dismissed" => Some(ReportStatus::Dismissed),
            _ => None,
        }
    }
}

/// Where a stored message copy came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentSource {
    /// Read from the messages service when the report was filed.
    Server,
    /// Submitted by the reporter: the server never stores ephemeral or P2P content.
    Reporter,
}

impl ContentSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            ContentSource::Server => "server",
            ContentSource::Reporter => "reporter",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "server" => Some(ContentSource::Server),
            "reporter" => Some(ContentSource::Reporter),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub id: Snowflake,
    pub reporter_id: Snowflake,
    pub reported_user_id: Snowflake,
    pub kind: ReportKind,
    pub category: ReportCategory,
    pub note: Option<String>,
    pub channel_id: Option<Snowflake>,
    pub guild_id: Option<Snowflake>,
    pub message_id: Option<Snowflake>,
    pub delivery_mode: Option<DeliveryMode>,
    pub content: Option<String>,
    pub content_source: Option<ContentSource>,
    pub status: ReportStatus,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolved_by: Option<String>,
    pub resolution_note: Option<String>,
}

/// Trims an optional free-text note; blank means none.
pub fn clean_note(note: Option<&str>) -> Result<Option<String>, FieldError> {
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    match note {
        Some(note) if note.chars().count() > MAX_NOTE_CHARS => Err(invalid(
            "note",
            "BASE_TYPE_BAD_LENGTH",
            "Must be 500 or fewer in length.",
        )),
        other => Ok(other.map(str::to_owned)),
    }
}

/// Validates the message text a reporter submits for content the server does not store.
pub fn clean_submitted_content(content: Option<&str>) -> Result<Option<String>, FieldError> {
    let content = content.map(str::trim).filter(|c| !c.is_empty());
    match content {
        Some(content) if content.chars().count() > MAX_SUBMITTED_CONTENT_CHARS => Err(invalid(
            "content",
            "BASE_TYPE_BAD_LENGTH",
            "Must be 4000 or fewer in length.",
        )),
        other => Ok(other.map(str::to_owned)),
    }
}

/// How a reviewer closes a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Resolved,
    Dismissed,
}

impl Resolution {
    pub const fn status(self) -> ReportStatus {
        match self {
            Resolution::Resolved => ReportStatus::Resolved,
            Resolution::Dismissed => ReportStatus::Dismissed,
        }
    }

    pub fn parse(raw: &str) -> Result<Self, FieldError> {
        match raw {
            "resolved" => Ok(Resolution::Resolved),
            "dismissed" => Ok(Resolution::Dismissed),
            _ => Err(invalid(
                "resolution",
                "REPORT_RESOLUTION_INVALID",
                "Must be resolved or dismissed.",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_category_round_trips_through_its_wire_name() {
        for category in ReportCategory::ALL {
            assert_eq!(ReportCategory::parse(category.as_str()).unwrap(), category);
        }
        assert!(ReportCategory::parse("annoying").is_err());
        assert!(ReportCategory::parse("").is_err());
    }

    #[test]
    fn notes_are_trimmed_and_bounded_by_characters() {
        assert_eq!(clean_note(None).unwrap(), None);
        assert_eq!(clean_note(Some("   ")).unwrap(), None);
        assert_eq!(clean_note(Some(" rude ")).unwrap(), Some("rude".into()));
        assert!(clean_note(Some(&"\u{e9}".repeat(500))).is_ok());
        assert!(clean_note(Some(&"\u{e9}".repeat(501))).is_err());
    }

    #[test]
    fn submitted_content_is_bounded() {
        assert!(clean_submitted_content(Some(&"a".repeat(4000))).is_ok());
        assert!(clean_submitted_content(Some(&"a".repeat(4001))).is_err());
        assert_eq!(clean_submitted_content(Some("  ")).unwrap(), None);
    }

    #[test]
    fn resolutions_map_to_closed_statuses() {
        assert_eq!(
            Resolution::parse("resolved").unwrap().status(),
            ReportStatus::Resolved
        );
        assert_eq!(
            Resolution::parse("dismissed").unwrap().status(),
            ReportStatus::Dismissed
        );
        assert!(Resolution::parse("open").is_err());
    }

    #[test]
    fn statuses_kinds_and_sources_parse_their_wire_names() {
        for status in [
            ReportStatus::Open,
            ReportStatus::Resolved,
            ReportStatus::Dismissed,
        ] {
            assert_eq!(ReportStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(ReportKind::parse("message"), Some(ReportKind::Message));
        assert_eq!(ReportKind::parse("channel"), Some(ReportKind::Channel));
        assert_eq!(ContentSource::parse("server"), Some(ContentSource::Server));
        assert_eq!(ContentSource::parse("p2p"), None);
    }
}
