use serde_json::json;

use domain::application::Webhook;
use domain::audit::{AuditAction, AuditChange};
use domain::validation;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::support::audit::AuditContext;
use crate::support::images::{store_hashed, ImageUpload};
use crate::use_cases::webhooks::webhook_access::{require_hostable, require_manager};
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone, Default)]
pub struct UpdateWebhookInput {
    pub name: Option<String>,
    pub avatar: Option<Option<ImageUpload>>,
    pub channel_id: Option<Snowflake>,
}

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    webhook_id: Snowflake,
    input: UpdateWebhookInput,
    audit: AuditContext<'_>,
) -> AppResult<Webhook> {
    let (before, _ctx) = require_manager(ports, actor, webhook_id).await?;
    let mut webhook = before.clone();
    let mut changes = Vec::new();
    if let Some(name) = input.name {
        validation::webhook_name(&name)?;
        webhook.name = name.trim().to_owned();
        changes.extend(AuditChange::if_changed(
            "name",
            json!(before.name),
            json!(webhook.name),
        ));
    }
    if let Some(avatar) = input.avatar {
        webhook.avatar = match avatar {
            Some(image) => {
                Some(store_hashed(ports, &format!("avatars/{webhook_id}"), image).await?)
            }
            None => None,
        };
        changes.extend(AuditChange::if_changed(
            "avatar_hash",
            json!(before.avatar),
            json!(webhook.avatar),
        ));
    }
    if let Some(channel_id) = input.channel_id {
        let channel = ports
            .channels
            .find(channel_id)
            .await?
            .ok_or(AppError::NotFound(Resource::Channel))?;
        if channel.guild_id != Some(before.guild_id) {
            return Err(AppError::NotFound(Resource::Channel));
        }
        require_hostable(&channel)?;
        webhook.channel_id = channel_id;
        changes.extend(AuditChange::if_changed(
            "channel_id",
            json!(before.channel_id.to_string()),
            json!(channel_id.to_string()),
        ));
    }
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::UpdateWebhook(webhook.clone()));
    if !changes.is_empty() {
        writes.audit(audit.entry(
            ports,
            webhook.guild_id,
            AuditAction::WebhookUpdate,
            Some(webhook_id),
            changes,
        )?);
    }
    ports.unit_of_work.commit(writes).await?;
    Ok(webhook)
}
