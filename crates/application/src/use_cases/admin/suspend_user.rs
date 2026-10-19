use chrono::{DateTime, Utc};

use domain::user::Suspension;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::{AppError, AppResult, Resource};
use crate::write_set::{Mutation, WriteSet};

/// `POST /internal/v1/admin/users/{id}/suspend`: blocks logins and revokes
/// every live session. `until: None` suspends indefinitely.
pub async fn execute(
    ports: &Ports,
    user_id: Snowflake,
    reason: String,
    until: Option<DateTime<Utc>>,
) -> AppResult<()> {
    let reason = reason.trim().to_owned();
    if reason.is_empty() || reason.chars().count() > 512 {
        return Err(AppError::invalid(
            "reason",
            "BASE_TYPE_BAD_LENGTH",
            "Must be between 1 and 512 in length.",
        ));
    }
    let now = ports.clock.now();
    if until.is_some_and(|until| until <= now) {
        return Err(AppError::invalid(
            "until",
            "DATE_IN_PAST",
            "Must be in the future.",
        ));
    }
    ports
        .users
        .find(user_id)
        .await?
        .ok_or(AppError::NotFound(Resource::User))?;
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::SuspendUser {
        user_id,
        suspension: Suspension { reason, until },
    });
    ports.unit_of_work.commit(writes).await?;
    ports.sessions.revoke_all_for_user(user_id, now).await
}
