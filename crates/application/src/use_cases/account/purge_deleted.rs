use crate::context::Ports;
use crate::error::AppResult;
use crate::events::DomainEvent;
use crate::support::sessions;
use crate::write_set::{Mutation, WriteSet};

/// Accounts handled per run; the rest wait for the next tick.
const BATCH: u32 = 100;

/// Anonymizes accounts whose retention window has ended: the username becomes
/// `deleted_<id>`, email, names, avatar, credentials and tokens go. Messages
/// keep their author id and show the placeholder. Returns how many were purged.
pub async fn execute(ports: &Ports) -> AppResult<usize> {
    let now = ports.clock.now();
    let due = ports.users.list_due_deletions(now, BATCH).await?;
    for user_id in &due {
        let Some(mut user) = ports.users.find(*user_id).await? else {
            continue;
        };
        let username = format!("deleted_{}", user_id.as_i64());
        user.username.clone_from(&username);
        user.global_name = None;
        user.avatar = None;
        user.email = None;
        user.verified = false;
        user.mfa_enabled = false;
        let mut writes = WriteSet::new();
        writes
            .mutate(Mutation::AnonymizeUser {
                user_id: *user_id,
                username,
            })
            .emit(DomainEvent::UserUpdate(user));
        ports.unit_of_work.commit(writes).await?;
        sessions::revoke(ports, *user_id, None).await?;
    }
    Ok(due.len())
}
