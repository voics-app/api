use domain::user::User;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;

pub const DEFAULT_LIMIT: u32 = 50;
pub const MAX_LIMIT: u32 = 200;

#[derive(Debug, Clone)]
pub struct UserPage {
    pub users: Vec<User>,
    /// Cursor for the next page; `None` on the last page.
    pub next_after: Option<Snowflake>,
}

/// `GET /internal/v1/admin/users?query=&limit=&after=`.
pub async fn execute(
    ports: &Ports,
    query: Option<&str>,
    limit: Option<u32>,
    after: Option<Snowflake>,
) -> AppResult<UserPage> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let query = query.map(str::trim).filter(|q| !q.is_empty());
    let mut users = ports.users.search(query, limit + 1, after).await?;
    let next_after = if users.len() > limit as usize {
        users.truncate(limit as usize);
        users.last().map(|u| u.id)
    } else {
        None
    };
    Ok(UserPage { users, next_after })
}
