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

/// Whether `raw` names an upload made for `channel_id` by the presign step,
/// shaped `attachments/{channel_id}/{attachment_id}/{filename}`.
fn is_upload_key(raw: &str, channel_id: Snowflake) -> bool {
    let mut parts = raw.splitn(4, '/');
    let (Some("attachments"), Some(channel), Some(attachment), Some(filename)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    channel == channel_id.to_string()
        && attachment.parse::<Snowflake>().is_ok()
        && !filename.is_empty()
        && !filename.contains('/')
}

fn unknown_upload() -> AppError {
    AppError::invalid(
        "attachments",
        "UNKNOWN_UPLOAD_REFERENCE",
        "Unknown attachment upload.",
    )
}

/// Translates client attachment references into messages' upload-reference
/// shape, rejecting any that do not name an upload made to this channel.
fn upload_references(channel_id: Snowflake, raw: Value) -> AppResult<Value> {
    let Value::Array(references) = raw else {
        return Err(unknown_upload());
    };
    references
        .iter()
        .map(|reference| upload_reference(channel_id, reference))
        .collect::<AppResult<Vec<_>>>()
        .map(Value::Array)
}

fn upload_reference(channel_id: Snowflake, raw: &Value) -> AppResult<Value> {
    let object = raw.as_object().ok_or_else(unknown_upload)?;
    let uploaded = object
        .get("uploaded_filename")
        .and_then(Value::as_str)
        .filter(|key| is_upload_key(key, channel_id))
        .ok_or_else(unknown_upload)?;
    let mut reference = Map::new();
    reference.insert(
        "uploaded_filename".to_owned(),
        Value::String(uploaded.to_owned()),
    );
    for key in ["id", "filename"] {
        if let Some(value) = object.get(key).filter(|value| !value.is_null()) {
            reference.insert(key.to_owned(), value.clone());
        }
    }
    Ok(Value::Object(reference))
}

fn is_ephemeral(fields: &Map<String, Value>) -> bool {
    fields
        .get("flags")
        .and_then(Value::as_u64)
        .is_some_and(|flags| flags & FLAG_EPHEMERAL != 0)
}

/// A new message: sanitized `data` plus its origin. `needs_body` rejects
/// messages without content, embeds, components or attachments.
pub fn compose(
    channel_id: Snowflake,
    data: Option<Value>,
    origin: MessageOrigin,
    needs_body: bool,
) -> AppResult<Value> {
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
    if let Some(references) = fields.get_mut("attachments") {
        if !references.is_null() {
            *references = upload_references(channel_id, references.take())?;
        }
    }
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

/// A partial update to an existing message. Attachments are mapped exactly as
/// on create, so only uploads made to `channel_id` are accepted.
pub fn edit_patch(channel_id: Snowflake, data: Option<Value>) -> AppResult<Value> {
    let mut fields = keep_only(object(data)?, &EDIT_FIELDS);
    if let Some(references) = fields.get_mut("attachments") {
        if !references.is_null() {
            *references = upload_references(channel_id, references.take())?;
        }
    }
    validate(&fields)?;
    Ok(Value::Object(fields))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn channel() -> Snowflake {
        Snowflake::new(100)
    }

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
            channel(),
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
            channel(),
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
            channel(),
            Some(json!({"content": "hi", "username": "Custom"})),
            origin,
            true,
        )
        .unwrap();
        assert_eq!(custom["username"], "Custom");
    }

    #[test]
    fn message_references_pass_through() {
        let message = compose(
            channel(),
            Some(json!({
                "content": "re",
                "message_reference": {"message_id": "5"},
            })),
            bot_origin(),
            true,
        )
        .unwrap();
        assert_eq!(message["message_reference"], json!({"message_id": "5"}));
    }

    #[test]
    fn client_attachments_map_to_upload_references() {
        let message = compose(
            channel(),
            Some(json!({
                "content": "pic",
                "attachments": [{
                    "id": "5",
                    "filename": "cat.png",
                    "uploaded_filename": "attachments/100/5/cat.png",
                    "size": 12
                }],
            })),
            bot_origin(),
            true,
        )
        .unwrap();
        assert_eq!(
            message["attachments"],
            json!([{
                "id": "5",
                "filename": "cat.png",
                "uploaded_filename": "attachments/100/5/cat.png"
            }])
        );
    }

    #[test]
    fn attachments_without_a_known_upload_for_this_channel_are_rejected() {
        let rejected = [
            json!({"id": "5", "filename": "cat.png"}),
            json!({"uploaded_filename": "attachments/101/5/cat.png"}),
            json!({"uploaded_filename": "avatars/100/5/cat.png"}),
            json!({"uploaded_filename": "attachments/100/x/cat.png"}),
            json!({"uploaded_filename": "attachments/100/5/"}),
            json!({"uploaded_filename": "attachments/100/5/dir/cat.png"}),
            json!("attachments/100/5/cat.png"),
        ];
        for attachment in rejected {
            let result = compose(
                channel(),
                Some(json!({"content": "x", "attachments": [attachment.clone()]})),
                bot_origin(),
                true,
            );
            assert!(
                matches!(result, Err(AppError::Validation(_))),
                "{attachment} should be rejected with 50035"
            );
        }
    }

    #[test]
    fn ephemeral_replies_name_only_the_invoker() {
        let ephemeral = compose(
            channel(),
            Some(json!({"content": "x", "flags": 64})),
            bot_origin(),
            true,
        )
        .unwrap();
        assert_eq!(ephemeral["flags"], 64);
        assert_eq!(ephemeral["target_user_ids"], json!(["9"]));

        let public = compose(channel(), Some(json!({"content": "x"})), bot_origin(), true).unwrap();
        assert!(public.get("target_user_ids").is_none());
    }

    #[test]
    fn rejects_empty_and_oversized_bodies() {
        let origin = MessageOrigin::default();
        assert!(compose(channel(), Some(json!({})), origin.clone(), true).is_err());
        assert!(compose(channel(), Some(json!({})), origin.clone(), false).is_ok());
        let long = "x".repeat(MAX_CONTENT_CHARS + 1);
        assert!(compose(
            channel(),
            Some(json!({ "content": long })),
            origin.clone(),
            true
        )
        .is_err());
        assert!(compose(channel(), Some(json!("text")), origin, true).is_err());
    }

    #[test]
    fn edit_patch_drops_identity_fields() {
        let patch =
            edit_patch(channel(), Some(json!({"content": "new", "author_id": "9"}))).unwrap();
        assert_eq!(patch, json!({"content": "new"}));
    }

    #[test]
    fn edit_patch_maps_attachments_like_create() {
        let attachment = json!({
            "id": "5",
            "filename": "cat.png",
            "uploaded_filename": "attachments/100/5/cat.png",
            "size": 12
        });
        let patch = edit_patch(
            channel(),
            Some(json!({"content": "pic", "attachments": [attachment.clone()]})),
        )
        .unwrap();
        let created = compose(
            channel(),
            Some(json!({"content": "pic", "attachments": [attachment]})),
            bot_origin(),
            true,
        )
        .unwrap();
        assert_eq!(patch["attachments"], created["attachments"]);
        assert_eq!(
            patch["attachments"],
            json!([{
                "id": "5",
                "filename": "cat.png",
                "uploaded_filename": "attachments/100/5/cat.png"
            }])
        );
    }

    #[test]
    fn edit_patch_rejects_attachments_without_a_channel_upload() {
        let result = edit_patch(
            channel(),
            Some(json!({"attachments": [{"id": "5", "filename": "cat.png"}]})),
        );
        assert!(matches!(result, Err(AppError::Validation(_))));
    }
}
