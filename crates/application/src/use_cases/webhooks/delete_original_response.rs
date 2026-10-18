use domain::Snowflake;

use crate::context::Ports;
use crate::error::{AppError, AppResult, Resource};
use crate::use_cases::webhooks::webhook_access::followup_interaction;
use crate::write_set::{Mutation, WriteSet};

/// `DELETE /webhooks/{app_id}/{token}/messages/@original`.
pub async fn execute(ports: &Ports, application_id: Snowflake, token: &str) -> AppResult<()> {
    let interaction = followup_interaction(ports, application_id, token).await?;
    let message_id = interaction
        .original_message_id
        .ok_or(AppError::NotFound(Resource::Message))?;
    ports
        .messages
        .delete(interaction.channel_id, message_id)
        .await?;
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::UpdateInteraction {
        id: interaction.id,
        state: interaction.state,
        original_message_id: None,
    });
    ports.unit_of_work.commit(writes).await
}
