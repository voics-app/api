use serde_json::json;

use domain::audit::{AuditAction, AuditChange};
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::guild_context::GuildContext;
use crate::use_cases::emojis::emoji_rules::require_editable;
use crate::write_set::{Mutation, WriteSet};

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    emoji_id: Snowflake,
    audit: AuditContext<'_>,
) -> AppResult<()> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    let mut emojis = ports.emojis.list(guild_id).await?;
    let emoji = emojis
        .iter()
        .find(|e| e.id == emoji_id)
        .cloned()
        .ok_or(AppError::NotFound(Resource::Emoji))?;
    require_editable(&ctx, &emoji)?;
    emojis.retain(|e| e.id != emoji_id);
    let changes = vec![AuditChange::new("name", Some(json!(emoji.name)), None)];
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::DeleteEmoji { guild_id, emoji_id })
        .emit(DomainEvent::EmojisUpdate { guild_id, emojis })
        .audit(audit.entry(
            ports,
            guild_id,
            AuditAction::EmojiDelete,
            Some(emoji_id),
            changes,
        )?);
    ports.unit_of_work.commit(writes).await
}
