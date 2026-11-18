use domain::access::GuildAccess;
use domain::audit::AuditAction;
use domain::bot_authorization::{supported_scopes, BotConsent};
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
    let scopes = supported_scopes(&input.scope)?;
    let application = ports
        .applications
        .find(input.client_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Application))?;
    if !application.bot_public && application.owner_id != actor.user_id {
        return Err(AppError::NotFound(Resource::Application));
    }
    if application.bot_require_code_grant {
        return Err(AppError::invalid(
            "client_id",
            "CODE_GRANT_UNSUPPORTED",
            "This application requires an unsupported authorization-code flow.",
        ));
    }
    let ctx = GuildContext::load(ports, input.guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_GUILD)?;
    let guild_id = input.guild_id;
    let bot_id = application.bot_user_id;
    let revision = ports
        .applications
        .installation_revision(guild_id, application.id)
        .await?;
    let installation = ports
        .applications
        .installation(guild_id, application.id)
        .await?;
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::GuardApplicationInstallation {
        guild_id,
        application_id: application.id,
        bot_user_id: bot_id,
        expected_generation: installation.map(|i| i.generation),
        expected_revision: revision,
    });
    let mut consent = BotConsent {
        id: ports.ids.next_id()?,
        user_id: actor.user_id,
        guild_id,
        application_id: application.id,
        scopes,
        permissions: Permissions::NONE,
        authorized_at: ports.clock.now(),
    };
    if let Some(member) = ports.guilds.member(guild_id, bot_id).await? {
        consent.permissions = ctx.access().base(&member);
        if input.permissions != consent.permissions {
            return Err(AppError::Conflict {
                code: 50035,
                message: "The bot permissions changed. Review them and try again.".into(),
            });
        }
        writes.mutate(Mutation::RecordBotConsent(consent));
        return ports.unit_of_work.commit(writes).await;
    }
    if ports.guilds.ban(guild_id, bot_id).await?.is_some() {
        return Err(AppError::Forbidden(Denial::Banned));
    }
    let permissions = Permissions::from_bits_truncate(input.permissions.bits());
    ctx.require_grantable(permissions)?;

    let bot = require_user(ports, bot_id).await?;
    let mut member = Member::new(guild_id, bot_id, ports.clock.now());
    // Ordinary kicks can leave a managed role behind. A new installation never reuses it.
    let mut installed_roles: Vec<_> = ctx
        .roles
        .iter()
        .filter(|role| !(role.managed && role.bot_id == Some(bot_id)))
        .cloned()
        .collect();
    for role in ctx
        .roles
        .iter()
        .filter(|role| role.managed && role.bot_id == Some(bot_id))
    {
        writes
            .mutate(Mutation::DeleteRole {
                guild_id,
                role_id: role.id,
            })
            .emit(DomainEvent::RoleDelete {
                guild_id,
                role_id: role.id,
            });
    }
    if !permissions.is_empty() {
        let role = Role {
            id: ports.ids.next_id()?,
            name: application.name.clone(),
            position: 1,
            permissions,
            managed: true,
            bot_id: Some(bot_id),
            ..Role::everyone(guild_id, Permissions::NONE)
        };
        writes.mutate(Mutation::ShiftRolesForBot { guild_id });
        for other in installed_roles.iter_mut().filter(|r| !r.is_everyone()) {
            other.position += 1;
            writes.emit(DomainEvent::RoleUpdate(other.clone()));
        }
        member.role_ids.push(role.id);
        installed_roles.push(role.clone());
        writes
            .mutate(Mutation::InsertRole(role.clone()))
            .emit(DomainEvent::RoleCreate(role));
    }
    consent.permissions =
        GuildAccess::new(&ctx.guild, &installed_roles, ports.clock.now()).base(&member);
    let profile = MemberProfile {
        member: member.clone(),
        user: bot,
    };
    let mut snapshot = guild_snapshot(ports, guild_id).await?;
    let guild = snapshot.guild.clone();
    snapshot.members.push(profile.clone());
    snapshot.roles = installed_roles;
    snapshot.member_count += 1;
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: None,
    };
    writes
        .mutate(Mutation::InsertMember(member))
        .mutate(Mutation::RegisterApplicationInstallation {
            guild_id,
            application_id: application.id,
            bot_user_id: bot_id,
            installed_at: ports.clock.now(),
        })
        .mutate(Mutation::RecordBotConsent(consent))
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
