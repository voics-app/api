use serde_json::json;

use domain::audit::{AuditAction, AuditChange};
use domain::emoji::{Emoji, MAX_EMOJIS_PER_GUILD, MAX_EMOJI_BYTES};
use domain::permissions::Permissions;
use domain::validation;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::guild_context::GuildContext;
use crate::support::images::{store_at, ImageUpload};
use crate::use_cases::emojis::emoji_rules::validate_roles;
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone)]
pub struct CreateEmojiInput {
    pub name: String,
    pub image: ImageUpload,
    pub roles: Vec<Snowflake>,
}

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    input: CreateEmojiInput,
    audit: AuditContext<'_>,
) -> AppResult<Emoji> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    let permissions = ctx.permissions();
    if !permissions
        .intersects(Permissions::CREATE_GUILD_EXPRESSIONS | Permissions::MANAGE_GUILD_EXPRESSIONS)
    {
        return Err(AppError::missing_permissions());
    }
    validation::emoji_name(&input.name)?;
    validate_roles(&ctx, &input.roles)?;
    let mut emojis = ports.emojis.list(guild_id).await?;
    if emojis.len() >= MAX_EMOJIS_PER_GUILD {
        return Err(AppError::BadRequest {
            code: 30008,
            message: "Maximum number of emojis reached".into(),
        });
    }
    let emoji = Emoji {
        id: ports.ids.next_id()?,
        guild_id,
        name: input.name,
        creator_id: Some(actor.user_id),
        role_ids: input.roles,
        animated: input.image.is_animated(),
        available: true,
        require_colons: true,
        managed: false,
    };
    store_at(
        ports,
        &format!("emojis/{}", emoji.id),
        input.image,
        MAX_EMOJI_BYTES,
    )
    .await?;
    emojis.push(emoji.clone());
    let changes = vec![AuditChange::new("name", None, Some(json!(emoji.name)))];
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::InsertEmoji(emoji.clone()))
        .emit(DomainEvent::EmojisUpdate { guild_id, emojis })
        .audit(audit.entry(
            ports,
            guild_id,
            AuditAction::EmojiCreate,
            Some(emoji.id),
            changes,
        )?);
    ports.unit_of_work.commit(writes).await?;
    Ok(emoji)
}
