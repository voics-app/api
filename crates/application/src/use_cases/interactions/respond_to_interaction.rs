use serde_json::Value;

use domain::application::{CallbackKind, Interaction, InteractionError};
use domain::Snowflake;

use crate::context::Ports;
use crate::error::{AppError, AppResult, Resource};
use crate::events::DomainEvent;
use crate::support::message_payload::{compose, edit_patch, MessageOrigin};
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone)]
pub struct InteractionResponse {
    pub kind: u8,
    pub data: Option<Value>,
}

/// Loads an interaction by id and proves possession of its token.
pub async fn authenticate(
    ports: &Ports,
    interaction_id: Snowflake,
    token: &str,
) -> AppResult<Interaction> {
    let interaction = ports
        .interactions
        .find(interaction_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Interaction))?;
    if interaction.token_hash != ports.secrets.hash(token) {
        return Err(AppError::NotFound(Resource::Interaction));
    }
    Ok(interaction)
}

/// `POST /interactions/{id}/{token}/callback`: the initial response.
pub async fn execute(
    ports: &Ports,
    interaction_id: Snowflake,
    token: &str,
    response: InteractionResponse,
) -> AppResult<()> {
    let interaction = authenticate(ports, interaction_id, token).await?;
    let callback = CallbackKind::from_code(response.kind)
        .ok_or_else(|| AppError::invalid("type", "BASE_TYPE_CHOICES", "Unknown callback type."))?;
    let state = interaction.accept_callback(callback, ports.clock.now())?;
    let application = ports
        .applications
        .find(interaction.application_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Application))?;

    let origin = MessageOrigin {
        author_id: Some(application.bot_user_id),
        application_id: Some(application.id),
        interaction_id: Some(interaction.id),
        invoker_id: Some(interaction.user_id),
        ..Default::default()
    };
    let mut writes = WriteSet::new();
    let mut original_message_id = None;
    match callback {
        CallbackKind::ChannelMessageWithSource => {
            let body = compose(interaction.channel_id, response.data, origin, true)?;
            original_message_id = Some(
                ports
                    .messages
                    .create(interaction.channel_id, body)
                    .await?
                    .id,
            );
        }
        CallbackKind::UpdateMessage | CallbackKind::DeferredUpdateMessage => {
            let source = interaction
                .source_message_id
                .ok_or(InteractionError::InvalidCallback)?;
            if callback == CallbackKind::UpdateMessage {
                let patch = edit_patch(interaction.channel_id, response.data)?;
                ports
                    .messages
                    .edit(interaction.channel_id, source, patch)
                    .await?;
            }
            original_message_id = Some(source);
        }
        CallbackKind::AutocompleteResult | CallbackKind::Modal => {
            let event_type = if callback == CallbackKind::Modal {
                "INTERACTION_MODAL_CREATE"
            } else {
                "INTERACTION_AUTOCOMPLETE_RESULT"
            };
            writes.emit(DomainEvent::InteractionUserNotice {
                application_id: application.id,
                user_id: interaction.user_id,
                event_type,
                payload: serde_json::json!({
                    "interaction_id": interaction.id.to_string(),
                    "channel_id": interaction.channel_id.to_string(),
                    "data": response.data.unwrap_or(Value::Null),
                }),
            });
        }
        CallbackKind::Pong | CallbackKind::DeferredChannelMessageWithSource => {}
    }
    writes.mutate(Mutation::UpdateInteraction {
        id: interaction.id,
        state,
        original_message_id,
    });
    ports.unit_of_work.commit(writes).await
}
