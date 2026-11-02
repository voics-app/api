//! Server discovery: the opt-in public listing of a guild (api#74).

use chrono::{DateTime, Utc};

use crate::guild::Guild;
use crate::validation::FieldError;

/// Fixed category slugs; the client localises their labels.
pub const CATEGORIES: [&str; 8] = [
    "gaming",
    "music",
    "entertainment",
    "science_tech",
    "education",
    "art_creative",
    "community",
    "other",
];

pub const MAX_TAGS: usize = 5;
const TAG_LENGTH: std::ops::RangeInclusive<usize> = 2..=24;
/// Shortest description a listed guild may carry.
pub const MIN_DESCRIPTION_CHARS: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildDiscovery {
    pub category: String,
    pub tags: Vec<String>,
    pub listed_at: DateTime<Utc>,
}

/// Thresholds a guild must meet to be listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscoveryRules {
    pub min_members: u64,
    pub min_age: chrono::Duration,
}

/// Why a guild cannot be listed right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ineligibility {
    DescriptionMissing,
    TooFewMembers { required: u64 },
    TooYoung,
    NsfwChannels,
}

impl Ineligibility {
    pub const fn code(self) -> &'static str {
        match self {
            Self::DescriptionMissing => "DESCRIPTION_REQUIRED",
            Self::TooFewMembers { .. } => "MEMBER_COUNT_TOO_LOW",
            Self::TooYoung => "GUILD_TOO_NEW",
            Self::NsfwChannels => "NSFW_NOT_ALLOWED",
        }
    }
}

/// Every unmet requirement; empty means the guild may be listed.
pub fn ineligibilities(
    guild: &Guild,
    member_count: u64,
    has_nsfw_channel: bool,
    rules: DiscoveryRules,
    now: DateTime<Utc>,
) -> Vec<Ineligibility> {
    let mut out = Vec::new();
    let described = guild
        .description
        .as_deref()
        .is_some_and(|d| d.trim().chars().count() >= MIN_DESCRIPTION_CHARS);
    if !described {
        out.push(Ineligibility::DescriptionMissing);
    }
    if member_count < rules.min_members {
        out.push(Ineligibility::TooFewMembers {
            required: rules.min_members,
        });
    }
    if now - guild.created_at < rules.min_age {
        out.push(Ineligibility::TooYoung);
    }
    if has_nsfw_channel {
        out.push(Ineligibility::NsfwChannels);
    }
    out
}

pub fn category(value: &str) -> Result<String, FieldError> {
    let value = value.trim().to_lowercase();
    if CATEGORIES.contains(&value.as_str()) {
        return Ok(value);
    }
    Err(FieldError {
        field: "category".into(),
        code: "BASE_TYPE_CHOICES",
        message: format!("Must be one of: {}.", CATEGORIES.join(", ")),
    })
}

/// Lowercased, trimmed, deduplicated tags (letters, digits, `-`).
pub fn tags(values: &[String]) -> Result<Vec<String>, FieldError> {
    let mut out: Vec<String> = Vec::new();
    for raw in values {
        let tag = raw.trim().to_lowercase();
        let valid = TAG_LENGTH.contains(&tag.chars().count())
            && tag.chars().all(|c| c.is_alphanumeric() || c == '-');
        if !valid {
            return Err(FieldError {
                field: "tags".into(),
                code: "BASE_TYPE_BAD_LENGTH",
                message: "Tags are 2-24 letters, digits or dashes.".into(),
            });
        }
        if !out.contains(&tag) {
            out.push(tag);
        }
    }
    if out.len() > MAX_TAGS {
        return Err(FieldError {
            field: "tags".into(),
            code: "BASE_TYPE_MAX_LENGTH",
            message: format!("At most {MAX_TAGS} tags."),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Snowflake;

    fn guild(description: Option<&str>, created_at: DateTime<Utc>) -> Guild {
        Guild {
            id: Snowflake::new(1),
            name: "g".into(),
            icon: None,
            description: description.map(str::to_owned),
            owner_id: Snowflake::new(2),
            voice_region: None,
            verification_level: 0,
            default_message_notifications: 0,
            explicit_content_filter: 0,
            afk_channel_id: None,
            afk_timeout: 300,
            system_channel_id: None,
            created_at,
        }
    }

    const RULES: DiscoveryRules = DiscoveryRules {
        min_members: 5,
        min_age: chrono::Duration::days(7),
    };

    #[test]
    fn eligible_guild_has_no_blockers() {
        let now = Utc::now();
        let g = guild(
            Some("A friendly place to chat"),
            now - chrono::Duration::days(8),
        );
        assert!(ineligibilities(&g, 5, false, RULES, now).is_empty());
    }

    #[test]
    fn every_unmet_rule_is_reported() {
        let now = Utc::now();
        let g = guild(Some("short"), now);
        let blockers = ineligibilities(&g, 1, true, RULES, now);
        assert_eq!(
            blockers,
            vec![
                Ineligibility::DescriptionMissing,
                Ineligibility::TooFewMembers { required: 5 },
                Ineligibility::TooYoung,
                Ineligibility::NsfwChannels,
            ]
        );
    }

    #[test]
    fn category_must_be_known_and_is_normalised() {
        assert_eq!(category(" Gaming ").unwrap(), "gaming");
        assert!(category("memes").is_err());
    }

    #[test]
    fn tags_are_normalised_deduplicated_and_bounded() {
        let input = [
            "Rust".to_owned(),
            "rust".to_owned(),
            "open-source".to_owned(),
        ];
        assert_eq!(tags(&input).unwrap(), vec!["rust", "open-source"]);
        assert!(tags(&["a".to_owned()]).is_err());
        assert!(tags(&["has space".to_owned()]).is_err());
        let many: Vec<String> = (0..6).map(|i| format!("tag{i}")).collect();
        assert!(tags(&many).is_err());
    }
}
