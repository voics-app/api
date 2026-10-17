use domain::relationship::Relationship;
use domain::user::User;

use crate::context::{Actor, Ports};
use crate::error::AppResult;

/// Relationships with the target users resolved.
pub async fn execute(ports: &Ports, actor: &Actor) -> AppResult<Vec<(Relationship, User)>> {
    actor.require_user()?;
    let relationships = ports.relationships.list(actor.user_id).await?;
    let ids: Vec<_> = relationships.iter().map(|r| r.target_id).collect();
    let users = ports.users.find_many(&ids).await?;
    Ok(relationships
        .into_iter()
        .filter_map(|r| users.iter().find(|u| u.id == r.target_id).cloned().map(|u| (r, u)))
        .collect())
}
