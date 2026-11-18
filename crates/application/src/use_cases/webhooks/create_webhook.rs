use serde_json::json;

use domain::application::{Webhook, WebhookKind};
use domain::audit::{AuditAction, AuditChange};
use domain::permissions::Permissions;
use domain::validation;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::support::audit::AuditContext;
use crate::support::channel_access::ChannelAccess;
use crate::support::images::{store_hashed, ImageUpload};
use crate::use_cases::webhooks::webhook_access::require_hostable;
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone)]
pub struct CreateWebhookInput {
    pub name: String,
    pub avatar: Option<ImageUpload>,
}

/// Creates an incoming webhook. The raw token is returned only here.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    channel_id: Snowflake,
    input: CreateWebhookInput,
    audit: AuditContext<'_>,
) -> AppResult<(Webhook, String)> {
    let access = ChannelAccess::visible(ports, channel_id, actor.user_id).await?;
    access.require(Permissions::MANAGE_WEBHOOKS)?;
    require_hostable(&access.channel)?;
    let guild_id = access.guild()?.guild.id;
    validation::webhook_name(&input.name)?;

    let id = ports.ids.next_id()?;
    let avatar = match input.avatar {
        Some(image) => Some(store_hashed(ports, &format!("avatars/{id}"), image).await?),
        None => None,
    };
    let token = ports.secrets.token(48);
    let webhook = Webhook {
        id,
        kind: WebhookKind::Incoming,
        guild_id,
        channel_id,
        creator_id: Some(actor.user_id),
        name: input.name.trim().to_owned(),
        avatar,
        application_id: None,
        token_hash: ports.secrets.hash(&token),
    };
    let changes = vec![
        AuditChange::new("name", None, Some(json!(webhook.name))),
        AuditChange::new("channel_id", None, Some(json!(channel_id.to_string()))),
    ];
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::InsertWebhook(webhook.clone()))
        .audit(audit.entry(
            ports,
            guild_id,
            AuditAction::WebhookCreate,
            Some(id),
            changes,
        )?);
    ports
        .unit_of_work
        .commit(writes)
        .await
        .map_err(|e| match e {
            AppError::Conflict { .. } => AppError::BadRequest {
                code: 30007,
                message: "Maximum number of webhooks reached".into(),
            },
            other => other,
        })?;
    Ok((webhook, token))
}
