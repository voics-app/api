use super::app_access::{view, ApplicationView};
use crate::{
    context::{Actor, Ports},
    error::{AppError, AppResult, Resource},
    write_set::{Mutation, WriteSet},
};
use domain::{bot_authorization::BotAuthorization, guild::Guild, Snowflake};

pub struct AuthorizationView {
    pub authorization: BotAuthorization,
    pub application: ApplicationView,
    pub guild: Guild,
}

pub async fn list(ports: &Ports, actor: &Actor) -> AppResult<Vec<AuthorizationView>> {
    actor.require_user()?;
    let mut views = Vec::new();
    for authorization in ports.applications.authorizations(actor.user_id).await? {
        // A concurrent removal may invalidate a row between these read-side queries.
        let Some(application) = ports
            .applications
            .find(authorization.installation.application_id)
            .await?
        else {
            continue;
        };
        let Some(guild) = ports
            .guilds
            .find(authorization.installation.guild_id)
            .await?
        else {
            continue;
        };
        views.push(AuthorizationView {
            authorization,
            application: view(ports, application).await?,
            guild,
        });
    }
    Ok(views)
}

pub async fn revoke(ports: &Ports, actor: &Actor, id: Snowflake) -> AppResult<()> {
    actor.require_user()?;
    let authorization = ports
        .applications
        .authorization(actor.user_id, id)
        .await?
        .ok_or(AppError::NotFound(Resource::Application))?;
    let installation = authorization.installation;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::GuardApplicationInstallation {
            guild_id: installation.guild_id,
            application_id: installation.application_id,
            bot_user_id: installation.bot_user_id,
            expected_generation: Some(installation.generation),
            expected_revision: Some(installation.generation),
        })
        .mutate(Mutation::RevokeBotConsent {
            id,
            user_id: actor.user_id,
        });
    ports.unit_of_work.commit(writes).await
}
