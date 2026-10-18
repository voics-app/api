use serde_json::Value;

use domain::application::InteractionState;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::{AppError, AppResult, Resource};
use crate::ports::StoredMessage;
use crate::support::message_payload::{compose, edit_patch, MessageOrigin};
use crate::use_cases::webhooks::webhook_access::followup_interaction;
use crate::write_set::{Mutation, WriteSet};

/// `PATCH /webhooks/{app_id}/{token}/messages/@original`. After a deferred
/// callback the first edit creates the response message.
pub async fn execute(
    ports: &Ports,
    application_id: Snowflake,
    token: &str,
    data: Value,
) -> AppResult<StoredMessage> {
    let interaction = followup_interaction(ports, application_id, token).await?;
    if let Some(message_id) = interaction.original_message_id {
        return ports
            .messages
            .edit(interaction.channel_id, message_id, edit_patch(Some(data))?)
            .await;
    }
    let application = ports
        .applications
        .find(interaction.application_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Application))?;
    let origin = MessageOrigin {
        author_id: Some(application.bot_user_id),
        application_id: Some(application.id),
        interaction_id: Some(interaction.id),
        ..Default::default()
    };
    let message = ports
        .messages
        .create(interaction.channel_id, compose(Some(data), origin, true)?)
        .await?;
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::UpdateInteraction {
        id: interaction.id,
        state: InteractionState::Responded,
        original_message_id: Some(message.id),
    });
    ports.unit_of_work.commit(writes).await?;
    Ok(message)
}
