use domain::relationship::{Relationship, RelationshipType};
use domain::user::User;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::support::views::require_user;

/// A user plus what they have in common with the caller.
pub struct Profile {
    pub user: User,
    /// Shared servers with the target's nickname in each.
    pub mutual_guilds: Vec<(Snowflake, Option<String>)>,
    pub mutual_friends: Vec<User>,
}

fn friends(relationships: &[Relationship]) -> Vec<Snowflake> {
    relationships
        .iter()
        .filter(|r| r.kind == RelationshipType::Friend)
        .map(|r| r.target_id)
        .collect()
}

pub async fn execute(ports: &Ports, actor: &Actor, target: Snowflake) -> AppResult<Profile> {
    actor.require_user()?;
    let user = require_user(ports, target).await?;
    if target == actor.user_id {
        return Ok(Profile {
            user,
            mutual_guilds: Vec::new(),
            mutual_friends: Vec::new(),
        });
    }

    let mine = ports.guilds.list_for_user(actor.user_id).await?;
    let theirs = ports.guilds.list_for_user(target).await?;
    let mut mutual_guilds = Vec::new();
    for guild in mine.iter().filter(|g| theirs.iter().any(|t| t.id == g.id)) {
        let nick = ports
            .guilds
            .member(guild.id, target)
            .await?
            .and_then(|m| m.nick);
        mutual_guilds.push((guild.id, nick));
    }

    let my_friends = friends(&ports.relationships.list(actor.user_id).await?);
    let their_friends = friends(&ports.relationships.list(target).await?);
    let shared: Vec<Snowflake> = my_friends
        .into_iter()
        .filter(|id| their_friends.contains(id))
        .collect();
    let mutual_friends = ports.users.find_many(&shared).await?;

    Ok(Profile {
        user,
        mutual_guilds,
        mutual_friends,
    })
}
