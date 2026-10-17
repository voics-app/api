use serde_json::json;

use domain::audit::{AuditAction, AuditChange};
use domain::emoji::Emoji;
use domain::validation;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::guild_context::GuildContext;
use crate::use_cases::emojis::emoji_rules::{require_editable, validate_roles};
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone, Default)]
pub struct UpdateEmojiInput {
    pub name: Option<String>,
    pub roles: Option<Vec<Snowflake>>,
}

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    emoji_id: Snowflake,
    input: UpdateEmojiInput,
    audit: AuditContext<'_>,
) -> AppResult<Emoji> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    let mut emojis = ports.emojis.list(guild_id).await?;
    let emoji = emojis.iter_mut().find(|e| e.id == emoji_id).ok_or(AppError::NotFound(Resource::Emoji))?;
    require_editable(&ctx, emoji)?;
    let before = emoji.clone();
    if let Some(name) = input.name {
        validation::emoji_name(&name)?;
        emoji.name = name;
    }
    if let Some(roles) = input.roles {
        validate_roles(&ctx, &roles)?;
        emoji.role_ids = roles;
    }
    let updated = emoji.clone();
    let changes: Vec<AuditChange> =
        AuditChange::if_changed("name", json!(before.name), json!(updated.name)).into_iter().collect();
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpdateEmoji(updated.clone()))
        .emit(DomainEvent::EmojisUpdate { guild_id, emojis })
        .audit(audit.entry(ports, guild_id, AuditAction::EmojiUpdate, Some(emoji_id), changes)?);
    ports.unit_of_work.commit(writes).await?;
    Ok(updated)
}
