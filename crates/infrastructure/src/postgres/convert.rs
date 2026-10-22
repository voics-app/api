//! Column <-> domain value conversions shared by the repositories.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use domain::channel::ThreadMetadata;
use domain::delivery_mode::DeliveryMode;
use domain::permissions::Permissions;
use domain::user::CustomStatus;
use domain::Snowflake;

pub fn sf(raw: i64) -> Snowflake {
    Snowflake::from_i64(raw)
}

pub fn sf_opt(raw: Option<i64>) -> Option<Snowflake> {
    raw.map(Snowflake::from_i64)
}

pub fn sfs(raw: Vec<i64>) -> Vec<Snowflake> {
    raw.into_iter().map(Snowflake::from_i64).collect()
}

pub fn raw_ids(ids: &[Snowflake]) -> Vec<i64> {
    ids.iter().map(|id| id.as_i64()).collect()
}

pub fn bits(permissions: Permissions) -> i64 {
    permissions.bits() as i64
}

pub fn permissions(raw: i64) -> Permissions {
    Permissions::from_bits_truncate(raw as u64)
}

pub fn mode(raw: &str) -> DeliveryMode {
    raw.parse().unwrap_or_default()
}

/// Escapes `%`, `_` and `\` so user input matches literally inside ILIKE.
pub fn like_pattern(query: &str) -> String {
    let escaped: String = query
        .chars()
        .flat_map(|c| {
            if matches!(c, '%' | '_' | '\\') {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect();
    format!("%{escaped}%")
}

pub fn custom_status_json(status: &CustomStatus) -> Value {
    json!({
        "text": status.text,
        "emoji_name": status.emoji_name,
        "expires_at": status.expires_at.map(|at| at.to_rfc3339()),
    })
}

pub fn custom_status_from_json(value: Value) -> Option<CustomStatus> {
    let object = value.as_object()?;
    Some(CustomStatus {
        text: object
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_owned),
        emoji_name: object
            .get("emoji_name")
            .and_then(Value::as_str)
            .map(str::to_owned),
        expires_at: object
            .get("expires_at")
            .and_then(Value::as_str)
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc)),
    })
}

pub fn thread_metadata_json(meta: &ThreadMetadata) -> Value {
    json!({
        "archived": meta.archived,
        "auto_archive_duration": meta.auto_archive_duration,
        "archive_timestamp": meta.archive_timestamp.to_rfc3339(),
        "locked": meta.locked,
        "invitable": meta.invitable,
        "create_timestamp": meta.create_timestamp.to_rfc3339(),
    })
}

pub fn thread_metadata_from_json(value: Value) -> Option<ThreadMetadata> {
    let at = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc))
    };
    Some(ThreadMetadata {
        archived: value.get("archived")?.as_bool()?,
        auto_archive_duration: value.get("auto_archive_duration")?.as_u64()? as u32,
        archive_timestamp: at("archive_timestamp")?,
        locked: value.get("locked")?.as_bool()?,
        invitable: value.get("invitable")?.as_bool()?,
        create_timestamp: at("create_timestamp")?,
    })
}

pub fn guild_folders_json(folders: &[domain::user::GuildFolder]) -> Value {
    Value::Array(
        folders
            .iter()
            .map(|f| {
                json!({
                    "id": f.id,
                    "name": f.name,
                    "color": f.color,
                    "guild_ids": f.guild_ids.iter().map(|g| g.to_string()).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

pub fn guild_folders_from_json(value: &Value) -> Vec<domain::user::GuildFolder> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| domain::user::GuildFolder {
                    id: item.get("id").and_then(Value::as_i64),
                    name: item.get("name").and_then(Value::as_str).map(str::to_owned),
                    color: item.get("color").and_then(Value::as_u64).map(|c| c as u32),
                    guild_ids: item
                        .get("guild_ids")
                        .and_then(Value::as_array)
                        .map(|ids| {
                            ids.iter()
                                .filter_map(Value::as_str)
                                .filter_map(|id| id.parse().ok())
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_patterns_escape_wildcards() {
        assert_eq!(like_pattern("a_b%c\\"), "%a\\_b\\%c\\\\%");
        assert_eq!(like_pattern("plain"), "%plain%");
    }

    #[test]
    fn thread_metadata_round_trips() {
        let now = Utc::now();
        let meta = ThreadMetadata {
            archived: true,
            auto_archive_duration: 1440,
            archive_timestamp: now,
            locked: false,
            invitable: true,
            create_timestamp: now,
        };
        let back = thread_metadata_from_json(thread_metadata_json(&meta)).unwrap();
        assert_eq!(back.archived, meta.archived);
        assert_eq!(back.auto_archive_duration, 1440);
        assert_eq!(
            back.archive_timestamp.timestamp_millis(),
            now.timestamp_millis()
        );
    }

    #[test]
    fn custom_status_round_trips() {
        let status = CustomStatus {
            text: Some("brb".into()),
            emoji_name: None,
            expires_at: None,
        };
        assert_eq!(
            custom_status_from_json(custom_status_json(&status)),
            Some(status)
        );
    }

    #[test]
    fn permission_bits_round_trip() {
        assert_eq!(
            permissions(bits(Permissions::DEFAULT_EVERYONE)),
            Permissions::DEFAULT_EVERYONE
        );
    }
}
