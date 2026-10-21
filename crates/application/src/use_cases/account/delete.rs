use domain::account::AccountState;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::support::credentials::confirm_password;
use crate::support::sessions;
use crate::write_set::{Mutation, WriteSet};

/// Discord's code for "transfer ownership of your guilds first".
const OWNS_GUILDS: u32 = 20011;

/// Schedules deletion after the retention window and signs the account out
/// everywhere. Logging in before the window ends cancels it; afterwards
/// [`super::purge_deleted`] anonymizes the account.
pub async fn execute(ports: &Ports, actor: &Actor, password: &str) -> AppResult<()> {
    actor.require_user()?;
    confirm_password(ports, actor.user_id, password).await?;
    let owns_guilds = ports
        .guilds
        .list_for_user(actor.user_id)
        .await?
        .iter()
        .any(|guild| guild.owner_id == actor.user_id);
    if owns_guilds {
        return Err(AppError::BadRequest {
            code: OWNS_GUILDS,
            message: "You must transfer ownership of any guilds you own first.".into(),
        });
    }
    let now = ports.clock.now();
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::SetAccountState {
        user_id: actor.user_id,
        state: AccountState {
            disabled_at: Some(now),
            delete_after: Some(now + ports.settings.account_deletion_retention),
        },
    });
    ports.unit_of_work.commit(writes).await?;
    sessions::revoke(ports, actor.user_id, None).await
}
