use crate::{
    context::{Actor, Ports},
    error::{AppError, AppResult, Resource},
    events::DomainEvent,
    support::{audit::AuditContext, guild_context::GuildContext},
    write_set::{Mutation, WriteSet},
};
use domain::{audit::AuditAction, ban_appeal::BanAppeal, permissions::Permissions, Snowflake};

fn valid(field: &'static str, value: &str, max: usize) -> AppResult<()> {
    if value.trim().is_empty() || value.chars().count() > max {
        return Err(AppError::invalid(
            field,
            "INVALID_LENGTH",
            format!("Must contain 1 to {max} characters."),
        ));
    }
    Ok(())
}
async fn moderator(ports: &Ports, actor: &Actor, guild: Snowflake) -> AppResult<()> {
    GuildContext::load(ports, guild, actor.user_id)
        .await?
        .require(Permissions::MANAGE_BAN_APPEALS)
}
pub async fn form(ports: &Ports, actor: &Actor, guild: Snowflake) -> AppResult<String> {
    actor.require_user()?;
    if ports.guilds.ban(guild, actor.user_id).await?.is_none() {
        moderator(ports, actor, guild).await?;
    }
    ports
        .guilds
        .appeal_form(guild)
        .await?
        .ok_or(AppError::NotFound(Resource::Guild))
}
pub async fn configure(
    ports: &Ports,
    actor: &Actor,
    guild: Snowflake,
    question: &str,
) -> AppResult<()> {
    moderator(ports, actor, guild).await?;
    valid("question", question, 500)?;
    ports.guilds.set_appeal_form(guild, question.trim()).await
}
pub async fn submit(
    ports: &Ports,
    actor: &Actor,
    guild: Snowflake,
    answer: &str,
) -> AppResult<BanAppeal> {
    actor.require_user()?;
    valid("answer", answer, 2000)?;
    if ports.guilds.ban(guild, actor.user_id).await?.is_none() {
        return Err(AppError::missing_access());
    }
    ports
        .guilds
        .submit_appeal(ports.ids.next_id()?, guild, actor.user_id, answer.trim())
        .await?
        .ok_or_else(|| AppError::Conflict {
            code: 0,
            message: "Appeal unavailable or already open".into(),
        })
}
pub async fn list(ports: &Ports, actor: &Actor, guild: Snowflake) -> AppResult<Vec<BanAppeal>> {
    moderator(ports, actor, guild).await?;
    ports.guilds.appeals(guild).await
}
pub async fn resolve(
    ports: &Ports,
    actor: &Actor,
    guild: Snowflake,
    id: Snowflake,
    accept: bool,
    audit: AuditContext<'_>,
) -> AppResult<()> {
    moderator(ports, actor, guild).await?;
    let appeal = ports
        .guilds
        .appeal(guild, id)
        .await?
        .ok_or(AppError::NotFound(Resource::Ban))?;
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::ResolveBanAppeal {
        guild_id: guild,
        appeal_id: id,
        user_id: appeal.user_id,
        reviewer_id: actor.user_id,
        accept,
    });
    if accept {
        let ban = ports
            .guilds
            .ban(guild, appeal.user_id)
            .await?
            .ok_or(AppError::NotFound(Resource::Ban))?;
        writes.emit(DomainEvent::BanRemove {
            guild_id: guild,
            user: ban.user,
        });
    }
    writes.audit(audit.entry(
        ports,
        guild,
        if accept {
            AuditAction::BanAppealAccept
        } else {
            AuditAction::BanAppealDeny
        },
        Some(appeal.user_id),
        vec![],
    )?);
    ports.unit_of_work.commit(writes).await
}
