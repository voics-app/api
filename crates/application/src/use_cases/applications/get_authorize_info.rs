use domain::guild::Guild;
use domain::permissions::Permissions;
use domain::user::User;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::support::guild_context::GuildContext;
use crate::support::views::require_user;
use crate::use_cases::applications::app_access::{view, ApplicationView};

#[derive(Debug, Clone)]
pub struct AuthorizeInfo {
    pub application: ApplicationView,
    pub user: User,
    /// Guilds where the caller may add the bot (MANAGE_GUILD), with permissions.
    pub guilds: Vec<(Guild, Permissions, Option<Permissions>)>,
    pub scopes: Vec<String>,
    pub permissions: Permissions,
}

/// `GET /oauth2/authorize?client_id=...` — what the bot-invite screen shows.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    client_id: Snowflake,
    scope: &str,
    permissions: Permissions,
) -> AppResult<AuthorizeInfo> {
    actor.require_user()?;
    let scopes = domain::bot_authorization::supported_scopes(scope)?;
    let application = ports
        .applications
        .find(client_id)
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
    let mut guilds = Vec::new();
    for guild in ports.guilds.list_for_user(actor.user_id).await? {
        let ctx = GuildContext::load(ports, guild.id, actor.user_id).await?;
        let permissions = ctx.permissions();
        if permissions.contains(Permissions::MANAGE_GUILD) {
            let bot_permissions = ports
                .guilds
                .member(guild.id, application.bot_user_id)
                .await?
                .map(|bot| ctx.access().base(&bot));
            guilds.push((ctx.guild, permissions, bot_permissions));
        }
    }
    Ok(AuthorizeInfo {
        application: view(ports, application).await?,
        user: require_user(ports, actor.user_id).await?,
        guilds,
        scopes,
        permissions,
    })
}
