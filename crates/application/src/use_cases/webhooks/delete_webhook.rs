use serde_json::json;

use domain::audit::{AuditAction, AuditChange};
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::support::audit::AuditContext;
use crate::use_cases::webhooks::webhook_access::require_manager;
use crate::write_set::{Mutation, WriteSet};

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    webhook_id: Snowflake,
    audit: AuditContext<'_>,
) -> AppResult<()> {
    let (webhook, _ctx) = require_manager(ports, actor, webhook_id).await?;
    let changes = vec![AuditChange::new("name", Some(json!(webhook.name)), None)];
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::DeleteWebhook(webhook_id))
        .audit(audit.entry(
            ports,
            webhook.guild_id,
            AuditAction::WebhookDelete,
            Some(webhook_id),
            changes,
        )?);
    ports.unit_of_work.commit(writes).await
}
