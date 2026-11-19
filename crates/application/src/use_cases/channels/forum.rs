use crate::{
    context::Ports,
    error::{AppError, AppResult},
};
use domain::channel::{Channel, ChannelType};
use serde_json::{json, Value};

fn invalid(field: &str, message: &str) -> AppError {
    AppError::invalid(field, "BASE_TYPE_BAD_LENGTH", message)
}

pub fn validate_emoji(value: &Value, field: &str) -> AppResult<()> {
    if !value.is_object() {
        return Err(invalid(field, "Expected an emoji object."));
    }
    let emoji_id = value.get("emoji_id");
    let emoji_name = value.get("emoji_name");
    if emoji_id
        .is_some_and(|v| !v.is_null() && v.as_str().is_none_or(|id| id.parse::<u64>().is_err()))
        || emoji_name.is_some_and(|v| {
            !v.is_null()
                && v.as_str()
                    .is_none_or(|name| name.is_empty() || name.chars().count() > 32)
        })
        || (emoji_id.is_some_and(|v| !v.is_null()) && emoji_name.is_some_and(|v| !v.is_null()))
    {
        return Err(invalid(field, "Invalid emoji."));
    }
    Ok(())
}

pub fn normalize_tags(ports: &Ports, channel: &Channel, input: Value) -> AppResult<Value> {
    let tags = input
        .as_array()
        .ok_or_else(|| invalid("available_tags", "Expected a list of tags."))?;
    if tags.len() > 20 {
        return Err(invalid("available_tags", "At most 20 tags are allowed."));
    }
    let existing = channel
        .forum_settings
        .get("available_tags")
        .and_then(Value::as_array);
    let mut ids = std::collections::HashSet::new();
    let mut result = Vec::new();
    for tag in tags {
        validate_emoji(tag, "available_tags")?;
        if tag.get("moderated").is_some_and(|v| !v.is_boolean()) {
            return Err(invalid("available_tags", "Moderated must be a boolean."));
        }
        if tag
            .get("id")
            .is_some_and(|v| !v.is_null() && !v.is_string())
        {
            return Err(invalid("available_tags", "Tag ID must be a string."));
        }
        let name = tag.get("name").and_then(Value::as_str).unwrap_or("").trim();
        if name.is_empty() || name.chars().count() > 20 {
            return Err(invalid(
                "available_tags",
                "Tag names must be 1–20 characters.",
            ));
        }
        let id = match tag.get("id").and_then(Value::as_str) {
            None | Some("") => ports.ids.next_id()?.to_string(),
            Some(id)
                if existing.is_some_and(|old| {
                    old.iter()
                        .any(|t| t.get("id").and_then(Value::as_str) == Some(id))
                }) =>
            {
                id.to_owned()
            }
            _ => return Err(invalid("available_tags", "Unknown tag ID.")),
        };
        if !ids.insert(id.clone()) {
            return Err(invalid("available_tags", "Duplicate tag ID."));
        }
        result.push(json!({"id": id, "name": name, "moderated": tag.get("moderated").and_then(Value::as_bool).unwrap_or(false), "emoji_id": tag.get("emoji_id").cloned().unwrap_or(Value::Null), "emoji_name": tag.get("emoji_name").cloned().unwrap_or(Value::Null)}));
    }
    Ok(Value::Array(result))
}

pub fn validate_applied(parent: &Channel, tags: &[String], manages: bool) -> AppResult<()> {
    if parent.kind != ChannelType::GuildForum {
        return Err(invalid("applied_tags", "Tags require a forum channel."));
    }
    if tags.len() > 5
        || (tags.is_empty()
            && parent
                .forum_settings
                .get("require_tags")
                .and_then(Value::as_bool)
                .unwrap_or(false))
    {
        return Err(invalid(
            "applied_tags",
            "Select between one and five tags when tags are required.",
        ));
    }
    let available = parent
        .forum_settings
        .get("available_tags")
        .and_then(Value::as_array);
    let mut seen = std::collections::HashSet::new();
    for id in tags {
        if !seen.insert(id) {
            return Err(invalid("applied_tags", "Duplicate tag."));
        }
        let tag = available
            .and_then(|items| {
                items
                    .iter()
                    .find(|t| t.get("id").and_then(Value::as_str) == Some(id.as_str()))
            })
            .ok_or_else(|| invalid("applied_tags", "Unknown tag."))?;
        if tag
            .get("moderated")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            && !manages
        {
            return Err(AppError::missing_permissions());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use domain::Snowflake;

    fn forum() -> Channel {
        let mut channel = Channel::new(
            Snowflake::new(1),
            ChannelType::GuildForum,
            Some(Snowflake::new(2)),
            Utc::now(),
        );
        channel.forum_settings = json!({"require_tags": true, "available_tags": [
            {"id": "12", "name": "general", "moderated": false},
            {"id": "13", "name": "staff", "moderated": true}
        ]});
        channel
    }

    #[test]
    fn validates_emoji_shapes() {
        assert!(validate_emoji(&json!({"emoji_id": null, "emoji_name": "heart"}), "emoji").is_ok());
        assert!(validate_emoji(&json!({"emoji_id": "123", "emoji_name": null}), "emoji").is_ok());
        assert!(validate_emoji(&json!({"emoji_id": 123}), "emoji").is_err());
        assert!(
            validate_emoji(&json!({"emoji_id": "123", "emoji_name": "heart"}), "emoji").is_err()
        );
    }

    #[test]
    fn validates_required_known_and_moderated_tags() {
        let channel = forum();
        assert!(validate_applied(&channel, &[], false).is_err());
        assert!(validate_applied(&channel, &["12".into()], false).is_ok());
        assert!(validate_applied(&channel, &["12".into(), "12".into()], false).is_err());
        assert!(validate_applied(&channel, &["999".into()], false).is_err());
        assert!(validate_applied(&channel, &["13".into()], false).is_err());
        assert!(validate_applied(&channel, &["13".into()], true).is_ok());
    }
}
