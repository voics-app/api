use domain::application::{Webhook, WebhookKind};
use domain::audit::{AuditAction, AuditChange};
use domain::Snowflake;
use serde_json::json;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::support::audit::AuditContext;
use crate::use_cases::webhooks::webhook_access::require_manager;
use crate::write_set::{Mutation, WriteSet};

/// Returns a new execution credential once; the write set and audit contain no raw token.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    id: Snowflake,
    audit: AuditContext<'_>,
) -> AppResult<(Webhook, String)> {
    let (webhook, _) = require_manager(ports, actor, id).await?;
    if webhook.kind != WebhookKind::Incoming {
        return Err(AppError::NotFound(Resource::Webhook));
    }
    let token = ports.secrets.token(48);
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::SetWebhookToken {
            id,
            token_hash: ports.secrets.hash(&token),
        })
        .audit(audit.entry(
            ports,
            webhook.guild_id,
            AuditAction::WebhookUpdate,
            Some(id),
            vec![AuditChange::new("token_rotated", None, Some(json!(true)))],
        )?);
    ports.unit_of_work.commit(writes).await?;
    let committed = ports
        .webhooks
        .find(id)
        .await?
        .ok_or(AppError::NotFound(Resource::Webhook))?;
    Ok((committed, token))
}
