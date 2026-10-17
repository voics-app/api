use domain::emoji::Emoji;
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::error::{AppError, AppResult};
use crate::support::guild_context::GuildContext;

/// Managers may edit any emoji; creators may edit their own.
pub fn require_editable(ctx: &GuildContext, emoji: &Emoji) -> AppResult<()> {
    let permissions = ctx.permissions();
    let own = emoji.creator_id == Some(ctx.member.user_id) && permissions.contains(Permissions::CREATE_GUILD_EXPRESSIONS);
    if own || permissions.contains(Permissions::MANAGE_GUILD_EXPRESSIONS) {
        Ok(())
    } else {
        Err(AppError::missing_permissions())
    }
}

pub fn validate_roles(ctx: &GuildContext, role_ids: &[Snowflake]) -> AppResult<()> {
    if role_ids.iter().all(|id| ctx.roles.iter().any(|r| r.id == *id)) {
        Ok(())
    } else {
        Err(AppError::invalid("roles", "ROLE_INVALID", "Unknown role."))
    }
}
