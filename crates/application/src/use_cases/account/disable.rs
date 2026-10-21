use domain::account::AccountState;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::support::credentials::confirm_password;
use crate::support::sessions;
use crate::write_set::{Mutation, WriteSet};

/// Disables the caller's account and signs it out everywhere. Logging in
/// again reactivates it.
pub async fn execute(ports: &Ports, actor: &Actor, password: &str) -> AppResult<()> {
    actor.require_user()?;
    confirm_password(ports, actor.user_id, password).await?;
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::SetAccountState {
        user_id: actor.user_id,
        state: AccountState {
            disabled_at: Some(ports.clock.now()),
            delete_after: None,
        },
    });
    ports.unit_of_work.commit(writes).await?;
    sessions::revoke(ports, actor.user_id, None).await
}
