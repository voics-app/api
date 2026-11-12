//! `GET /internal/v1/users/{id}/push-tokens` and `GET /internal/v1/push-tokens`: the devices a
//! notification sent by hand from the admin portal goes to (through the notifier). Only
//! notification tokens on a live session count; PushKit tokens only ring for calls.

use domain::push_token::PushToken;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;

/// Users one page of the whole audience holds at most.
pub const MAX_PAGE: u32 = 500;
pub const DEFAULT_PAGE: u32 = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualTargets {
    /// Each user with at least one live token, and those tokens.
    pub users: Vec<(Snowflake, Vec<PushToken>)>,
    /// Pass it as `after` for the next page; `None` when this page was the last.
    pub next_after: Option<Snowflake>,
}

fn group(tokens: Vec<PushToken>) -> Vec<(Snowflake, Vec<PushToken>)> {
    let mut users: Vec<(Snowflake, Vec<PushToken>)> = Vec::new();
    for token in tokens {
        match users.last_mut() {
            Some((id, list)) if *id == token.user_id => list.push(token),
            _ => users.push((token.user_id, vec![token])),
        }
    }
    users
}

/// One user's live tokens; an unknown user or one without tokens gives an empty list.
pub async fn for_user(ports: &Ports, user_id: Snowflake) -> AppResult<ManualTargets> {
    let tokens = ports.push_tokens.list_live_for_users(&[user_id]).await?;
    Ok(ManualTargets {
        users: group(tokens),
        next_after: None,
    })
}

/// The next page of users with a live token, ordered by id.
pub async fn page(
    ports: &Ports,
    after: Option<Snowflake>,
    limit: Option<u32>,
) -> AppResult<ManualTargets> {
    let limit = limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let users = group(ports.push_tokens.list_live_page(after, limit).await?);
    let next_after = if users.len() as u32 >= limit {
        users.last().map(|(id, _)| *id)
    } else {
        None
    };
    Ok(ManualTargets { users, next_after })
}
