use domain::application::{Application, ApplicationCommand};
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::support::guild_context::GuildContext;

#[derive(Debug, Clone)]
pub struct CommandIndex {
    pub applications: Vec<Application>,
    pub commands: Vec<ApplicationCommand>,
}

/// `GET /guilds/{id}/application-command-index`: commands a member can
/// invoke (global and guild commands of installed bots).
pub async fn execute(ports: &Ports, actor: &Actor, guild_id: Snowflake) -> AppResult<CommandIndex> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::USE_APPLICATION_COMMANDS)?;
    let permissions = ctx.permissions();
    let applications = ports.applications.installed_in_guild(guild_id).await?;
    let mut commands = Vec::new();
    for application in &applications {
        for scope in [None, Some(guild_id)] {
            commands.extend(
                ports
                    .applications
                    .commands(application.id, scope)
                    .await?
                    .into_iter()
                    .filter(|c| {
                        c.default_member_permissions
                            .is_none_or(|p| permissions.contains(p))
                    }),
            );
        }
    }
    Ok(CommandIndex {
        applications,
        commands,
    })
}
