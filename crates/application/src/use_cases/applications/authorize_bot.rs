use domain::audit::AuditAction;
use domain::guild::{Member, MemberProfile, Role};
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Denial, Resource};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::guild_context::GuildContext;
use crate::support::system_messages;
use crate::support::views::{guild_snapshot, require_user};
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone)]
pub struct AuthorizeBotInput {
    pub client_id: Snowflake,
    pub guild_id: Snowflake,
    pub permissions: Permissions,
    pub scope: String,
}

/// `POST /oauth2/authorize` with scope `bot` (and optionally
/// `applications.commands`): adds the bot to the guild with a managed role.
pub async fn execute(ports: &Ports, actor: &Actor, input: AuthorizeBotInput) -> AppResult<()> {
    actor.require_user()?;
    let scopes: Vec<&str> = input.scope.split_whitespace().collect();
    if !scopes.contains(&"bot")
        || scopes
            .iter()
            .any(|s| !matches!(*s, "bot" | "applications.commands"))
    {
        return Err(AppError::invalid(
            "scope",
            "OAUTH2_SCOPE_INVALID",
            "Scope must include bot.",
        ));
    }
    let application = ports
        .applications
        .find(input.client_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Application))?;
    if !application.bot_public && application.owner_id != actor.user_id {
        return Err(AppError::NotFound(Resource::Application));
    }
    let ctx = GuildContext::load(ports, input.guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_GUILD)?;
    let guild_id = input.guild_id;
    let bot_id = application.bot_user_id;
    if ports.guilds.member(guild_id, bot_id).await?.is_some() {
        return Ok(());
    }
    if ports.guilds.ban(guild_id, bot_id).await?.is_some() {
        return Err(AppError::Forbidden(Denial::Banned));
    }
    let permissions = Permissions::from_bits_truncate(input.permissions.bits());
    ctx.require_grantable(permissions)?;

    let bot = require_user(ports, bot_id).await?;
    let mut member = Member::new(guild_id, bot_id, ports.clock.now());
    let mut writes = WriteSet::new();
    if !permissions.is_empty() {
        let role = Role {
            id: ports.ids.next_id(),
            name: application.name.clone(),
            position: 1,
            permissions,
            managed: true,
            bot_id: Some(bot_id),
            ..Role::everyone(guild_id, Permissions::NONE)
        };
        for mut other in ctx.roles.iter().filter(|r| !r.is_everyone()).cloned() {
            other.position += 1;
            writes
                .mutate(Mutation::UpdateRole(other.clone()))
                .emit(DomainEvent::RoleUpdate(other));
        }
        member.role_ids.push(role.id);
        writes
            .mutate(Mutation::InsertRole(role.clone()))
            .emit(DomainEvent::RoleCreate(role));
    }
    let profile = MemberProfile {
        member: member.clone(),
        user: bot,
    };
    let mut snapshot = guild_snapshot(ports, guild_id).await?;
    let guild = snapshot.guild.clone();
    snapshot.members.push(profile.clone());
    snapshot.member_count += 1;
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: None,
    };
    writes
        .mutate(Mutation::UpsertMember(member))
        .emit(DomainEvent::MemberAdd(profile))
        .emit(DomainEvent::GuildCreate {
            snapshot: Box::new(snapshot),
            recipients: vec![bot_id],
        })
        .audit(audit.entry(ports, guild_id, AuditAction::BotAdd, Some(bot_id), vec![])?);
    ports.unit_of_work.commit(writes).await?;
    system_messages::announce_join(ports, &guild, bot_id).await;
    Ok(())
}
