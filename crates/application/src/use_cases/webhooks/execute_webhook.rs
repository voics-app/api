use serde_json::Value;

use domain::Snowflake;

use crate::context::Ports;
use crate::error::{AppError, AppResult, Resource};
use crate::ports::StoredMessage;
use crate::support::message_payload::{compose, MessageOrigin};
use crate::use_cases::webhooks::webhook_access::{followup_interaction, incoming};

/// `POST /webhooks/{id}/{token}`: executes an incoming webhook, or posts an
/// interaction follow-up when `id` is an application id.
pub async fn execute(ports: &Ports, id: Snowflake, token: &str, data: Value) -> AppResult<StoredMessage> {
    if let Some(webhook) = incoming(ports, id, token).await? {
        let origin = MessageOrigin { webhook_id: Some(webhook.id), ..Default::default() };
        let mut body = compose(Some(data), origin, true)?;
        let fields = body.as_object_mut().ok_or_else(|| AppError::internal("message body is not an object"))?;
        fields.entry("username").or_insert_with(|| Value::String(webhook.name.clone()));
        if let Some(avatar) = &webhook.avatar {
            fields.insert("webhook_avatar".to_owned(), Value::String(avatar.clone()));
        }
        return ports.messages.create(webhook.channel_id, body).await;
    }
    let interaction = followup_interaction(ports, id, token).await?;
    let application =
        ports.applications.find(interaction.application_id).await?.ok_or(AppError::NotFound(Resource::Application))?;
    let origin = MessageOrigin {
        author_id: Some(application.bot_user_id),
        application_id: Some(application.id),
        interaction_id: Some(interaction.id),
        ..Default::default()
    };
    ports.messages.create(interaction.channel_id, compose(Some(data), origin, true)?).await
}
