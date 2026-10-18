//! `POST /internal/v1/users/lookup`: public profiles for a batch of ids, so
//! messages can hydrate message authors. Unknown ids are skipped.

use domain::user::User;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;

/// Most ids accepted in one lookup.
pub const LOOKUP_LIMIT: usize = 100;

pub async fn execute(ports: &Ports, ids: &[Snowflake]) -> AppResult<Vec<User>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    ports.users.find_many(ids).await
}
