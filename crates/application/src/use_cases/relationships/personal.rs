use super::put_relationship;
use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use chrono::{DateTime, Duration, Utc};
use domain::Snowflake;

pub async fn favourites(ports: &Ports, actor: &Actor) -> AppResult<Vec<Snowflake>> {
    actor.require_user()?;
    ports.relationships.favourites(actor.user_id).await
}

pub async fn set_favourite(
    ports: &Ports,
    actor: &Actor,
    target: Snowflake,
    value: bool,
) -> AppResult<()> {
    actor.require_user()?;
    if !ports
        .relationships
        .set_favourite(actor.user_id, target, value)
        .await?
    {
        return Err(AppError::NotFound(Resource::User));
    }
    Ok(())
}

pub async fn create_invite(ports: &Ports, actor: &Actor) -> AppResult<(String, DateTime<Utc>)> {
    actor.require_user()?;
    let token = ports.secrets.token(32);
    let expires_at = ports.clock.now() + Duration::days(1);
    ports
        .relationships
        .create_friend_invite(actor.user_id, &ports.secrets.hash(&token), expires_at)
        .await?;
    Ok((token, expires_at))
}

pub async fn revoke_invites(ports: &Ports, actor: &Actor) -> AppResult<()> {
    actor.require_user()?;
    ports
        .relationships
        .revoke_friend_invites(actor.user_id)
        .await
}

pub async fn use_invite(ports: &Ports, actor: &Actor, token: &str) -> AppResult<()> {
    actor.require_user()?;
    let owner = ports
        .relationships
        .friend_invite_owner(&ports.secrets.hash(token), ports.clock.now())
        .await?
        .ok_or(AppError::NotFound(Resource::Invite))?;
    put_relationship::execute(ports, actor, owner, None).await
}
