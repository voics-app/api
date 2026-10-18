//! Builds the message bodies api asks the `messages` service to store on
//! behalf of bots, webhooks and interaction responses. The body is exactly
//! the pinned P4 shape: `author_id`, `webhook_id?`, `username?`, `avatar_url?`,
//! `interaction_id?`, `application_id?`, `content?`, `embeds?`, `components?`,
//! `flags?`, `message_reference?`, `attachments?`, plus `target_user_ids?` for
//! ephemeral replies.

use serde_json::{Map, Value};

use domain::Snowflake;

use crate::error::{AppError, AppResult};

pub const MAX_CONTENT_CHARS: usize = 2000;
pub const MAX_EMBEDS: usize = 10;

/// Discord's ephemeral message flag.
const FLAG_EPHEMERAL: u64 = 1 << 6;

/// Fields a bot or webhook may set on a message it creates.
const CREATE_FIELDS: [&str; 6] = [
    "content",
    "embeds",
    "components",
    "attachments",
    "flags",
    "message_reference",
];
/// Display overrides, honoured for webhook posts only.
const WEBHOOK_FIELDS: [&str; 2] = ["username", "avatar_url"];
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
#[derive(Debug, Clone, Default)]
pub struct MessageOrigin {
    /// The bot user the message is posted as (interaction responses).
    pub author_id: Option<Snowflake>,
    pub application_id: Option<Snowflake>,
    /// Set for webhook posts; the webhook is then the author.
    pub webhook_id: Option<Snowflake>,
    /// The webhook's name, the default `username` for webhook posts.
    pub webhook_name: Option<String>,
    pub interaction_id: Option<Snowflake>,
    /// The user who invoked the interaction: the only recipient of an
    /// ephemeral reply.
    pub invoker_id: Option<Snowflake>,
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

fn is_ephemeral(fields: &Map<String, Value>) -> bool {
    fields
        .get("flags")
        .and_then(Value::as_u64)
        .is_some_and(|flags| flags & FLAG_EPHEMERAL != 0)
}

/// A new message: sanitized `data` plus its origin. `needs_body` rejects
/// messages without content, embeds, components or attachments.
pub fn compose(data: Option<Value>, origin: MessageOrigin, needs_body: bool) -> AppResult<Value> {
    let is_webhook = origin.webhook_id.is_some();
    let allowed: Vec<&str> = if is_webhook {
        CREATE_FIELDS
            .iter()
            .chain(WEBHOOK_FIELDS.iter())
            .copied()
            .collect()
    } else {
        CREATE_FIELDS.to_vec()
    };
    let mut fields = keep_only(object(data)?, &allowed);
    validate(&fields)?;
    if needs_body {
        require_body(&fields)?;
    }

    let mut body = Map::new();
    // Webhook posts are authored by the webhook itself.
    let author = origin.webhook_id.or(origin.author_id);
    let mut put = |key: &str, id: Option<Snowflake>| {
        if let Some(id) = id {
            body.insert(key.to_owned(), Value::String(id.to_string()));
        }
    };
    put("author_id", author);
    put("webhook_id", origin.webhook_id);
    put("application_id", origin.application_id);
    put("interaction_id", origin.interaction_id);
    if is_webhook && !fields.contains_key("username") {
        if let Some(name) = origin.webhook_name {
            fields.insert("username".to_owned(), Value::String(name));
        }
    }
    if is_ephemeral(&fields) {
        if let Some(invoker) = origin.invoker_id {
            body.insert(
                "target_user_ids".to_owned(),
                Value::Array(vec![Value::String(invoker.to_string())]),
            );
        }
    }
    body.extend(fields);
    Ok(Value::Object(body))
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

    fn bot_origin() -> MessageOrigin {
        MessageOrigin {
            author_id: Some(Snowflake::new(1)),
            application_id: Some(Snowflake::new(2)),
            interaction_id: Some(Snowflake::new(3)),
            invoker_id: Some(Snowflake::new(9)),
            ..Default::default()
        }
    }

    #[test]
    fn interaction_bodies_have_exactly_the_pinned_fields() {
        let message = compose(
            Some(json!({
                "content": "hi",
                "id": "forged",
                "pinned": true,
                "type": 4,
                "tts": true,
                "allowed_mentions": {"parse": []},
                "username": "ignored for bots",
            })),
            bot_origin(),
            true,
        )
        .unwrap();
        assert_eq!(
            message,
            json!({"author_id": "1", "application_id": "2", "interaction_id": "3", "content": "hi"})
        );
    }

    #[test]
    fn webhook_bodies_are_authored_by_the_webhook_with_overrides() {
        let origin = MessageOrigin {
            author_id: Some(Snowflake::new(8)),
            webhook_id: Some(Snowflake::new(7)),
            webhook_name: Some("Relay".into()),
            ..Default::default()
        };
        let message = compose(
            Some(json!({"content": "hi", "avatar_url": "https://cdn.test/a.png"})),
            origin.clone(),
            true,
        )
        .unwrap();
        assert_eq!(
            message,
            json!({
                "author_id": "7",
                "webhook_id": "7",
                "username": "Relay",
                "avatar_url": "https://cdn.test/a.png",
                "content": "hi"
            })
        );

        let custom = compose(
            Some(json!({"content": "hi", "username": "Custom"})),
            origin,
            true,
        )
        .unwrap();
        assert_eq!(custom["username"], "Custom");
    }

    #[test]
    fn references_and_attachments_pass_through() {
        let message = compose(
            Some(json!({
                "content": "re",
                "message_reference": {"message_id": "5"},
                "attachments": [{"uploaded_filename": "attachments/1/2/a.png"}],
            })),
            bot_origin(),
            true,
        )
        .unwrap();
        assert_eq!(message["message_reference"], json!({"message_id": "5"}));
        assert_eq!(
            message["attachments"][0]["uploaded_filename"],
            "attachments/1/2/a.png"
        );
    }

    #[test]
    fn ephemeral_replies_name_only_the_invoker() {
        let ephemeral = compose(
            Some(json!({"content": "x", "flags": 64})),
            bot_origin(),
            true,
        )
        .unwrap();
        assert_eq!(ephemeral["flags"], 64);
        assert_eq!(ephemeral["target_user_ids"], json!(["9"]));

        let public = compose(Some(json!({"content": "x"})), bot_origin(), true).unwrap();
        assert!(public.get("target_user_ids").is_none());
    }

    #[test]
    fn rejects_empty_and_oversized_bodies() {
        let origin = MessageOrigin::default();
        assert!(compose(Some(json!({})), origin.clone(), true).is_err());
        assert!(compose(Some(json!({})), origin.clone(), false).is_ok());
        let long = "x".repeat(MAX_CONTENT_CHARS + 1);
        assert!(compose(Some(json!({ "content": long })), origin.clone(), true).is_err());
        assert!(compose(Some(json!("text")), origin, true).is_err());
    }

    #[test]
    fn edit_patch_drops_identity_fields() {
        let patch = edit_patch(Some(json!({"content": "new", "author_id": "9"}))).unwrap();
        assert_eq!(patch, json!({"content": "new"}));
    }
}
