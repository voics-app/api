//! Builds the message bodies api asks the `messages` service to store on
//! behalf of bots, webhooks and interaction responses.

use serde_json::{Map, Value};

use domain::Snowflake;

use crate::error::{AppError, AppResult};

pub const MAX_CONTENT_CHARS: usize = 2000;
pub const MAX_EMBEDS: usize = 10;

/// Message `type` for a reply to an application command (CHAT_INPUT_COMMAND).
const MESSAGE_TYPE_CHAT_INPUT_COMMAND: u8 = 20;

/// Fields a webhook or bot may set on a message it creates.
const CREATE_FIELDS: [&str; 9] = [
    "content",
    "embeds",
    "components",
    "attachments",
    "tts",
    "allowed_mentions",
    "flags",
    "username",
    "avatar_url",
];
/// Fields that may change on an edit.
const EDIT_FIELDS: [&str; 6] = [
    "content",
    "embeds",
    "components",
    "attachments",
    "allowed_mentions",
    "flags",
];

/// Who a created message is attributed to.
#[derive(Debug, Clone, Copy, Default)]
pub struct MessageOrigin {
    pub author_id: Option<Snowflake>,
    pub application_id: Option<Snowflake>,
    pub webhook_id: Option<Snowflake>,
    pub interaction_id: Option<Snowflake>,
}

fn object(data: Option<Value>) -> AppResult<Map<String, Value>> {
    match data {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map),
        Some(_) => Err(AppError::invalid(
            "data",
            "BASE_TYPE_OBJECT",
            "Expected an object.",
        )),
    }
}

fn validate(fields: &Map<String, Value>) -> AppResult<()> {
    if let Some(content) = fields.get("content").and_then(Value::as_str) {
        if content.chars().count() > MAX_CONTENT_CHARS {
            return Err(AppError::invalid(
                "content",
                "BASE_TYPE_MAX_LENGTH",
                "Must be 2000 or fewer in length.",
            ));
        }
    }
    if fields
        .get("embeds")
        .and_then(Value::as_array)
        .is_some_and(|e| e.len() > MAX_EMBEDS)
    {
        return Err(AppError::invalid(
            "embeds",
            "BASE_TYPE_MAX_LENGTH",
            "Must be 10 or fewer in length.",
        ));
    }
    Ok(())
}

fn keep_only(map: Map<String, Value>, allowed: &[&str]) -> Map<String, Value> {
    map.into_iter()
        .filter(|(key, _)| allowed.contains(&key.as_str()))
        .collect()
}

fn require_body(fields: &Map<String, Value>) -> AppResult<()> {
    let has = |key: &str| {
        fields.get(key).is_some_and(|v| match v {
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Null => false,
            _ => true,
        })
    };
    if has("content") || has("embeds") || has("components") || has("attachments") {
        Ok(())
    } else {
        Err(AppError::BadRequest {
            code: 50006,
            message: "Cannot send an empty message".into(),
        })
    }
}

/// A new message: sanitized `data` plus its origin. `needs_body` rejects
/// messages without content, embeds, components or attachments.
pub fn compose(data: Option<Value>, origin: MessageOrigin, needs_body: bool) -> AppResult<Value> {
    let mut fields = keep_only(object(data)?, &CREATE_FIELDS);
    validate(&fields)?;
    if needs_body {
        require_body(&fields)?;
    }
    let mut put = |key: &str, id: Option<Snowflake>| {
        if let Some(id) = id {
            fields.insert(key.to_owned(), Value::String(id.to_string()));
        }
    };
    put("author_id", origin.author_id);
    put("application_id", origin.application_id);
    put("webhook_id", origin.webhook_id);
    put("interaction_id", origin.interaction_id);
    if origin.interaction_id.is_some() {
        fields.insert(
            "type".to_owned(),
            Value::from(MESSAGE_TYPE_CHAT_INPUT_COMMAND),
        );
    }
    Ok(Value::Object(fields))
}

/// A partial update to an existing message.
pub fn edit_patch(data: Option<Value>) -> AppResult<Value> {
    let fields = keep_only(object(data)?, &EDIT_FIELDS);
    validate(&fields)?;
    Ok(Value::Object(fields))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compose_keeps_allowed_fields_and_stamps_origin() {
        let origin = MessageOrigin {
            author_id: Some(Snowflake::new(1)),
            application_id: Some(Snowflake::new(2)),
            interaction_id: Some(Snowflake::new(3)),
            ..Default::default()
        };
        let message = compose(
            Some(json!({"content": "hi", "id": "forged", "pinned": true})),
            origin,
            true,
        )
        .unwrap();
        assert_eq!(
            message,
            json!({"content": "hi", "author_id": "1", "application_id": "2", "interaction_id": "3", "type": 20})
        );
    }

    #[test]
    fn rejects_empty_and_oversized_bodies() {
        let origin = MessageOrigin::default();
        assert!(compose(Some(json!({})), origin, true).is_err());
        assert!(compose(Some(json!({})), origin, false).is_ok());
        let long = "x".repeat(MAX_CONTENT_CHARS + 1);
        assert!(compose(Some(json!({ "content": long })), origin, true).is_err());
        assert!(compose(Some(json!("text")), origin, true).is_err());
    }

    #[test]
    fn edit_patch_drops_identity_fields() {
        let patch = edit_patch(Some(json!({"content": "new", "author_id": "9"}))).unwrap();
        assert_eq!(patch, json!({"content": "new"}));
    }
}
