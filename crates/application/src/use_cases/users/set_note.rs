use domain::notification::validate_note;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::events::DomainEvent;
use crate::support::views::require_user;
use crate::write_set::{Mutation, WriteSet};

/// Sets the caller's private note about `target_id`; an empty note (or `None`) deletes it.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    target_id: Snowflake,
    note: Option<String>,
) -> AppResult<()> {
    actor.require_user()?;
    let note = note.filter(|n| !n.is_empty());
    if let Some(note) = &note {
        validate_note(note)?;
    }
    require_user(ports, target_id).await?;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::SetUserNote {
            owner_id: actor.user_id,
            target_id,
            note: note.clone(),
        })
        .emit(DomainEvent::UserNoteUpdate {
            owner_id: actor.user_id,
            target_id,
            note: note.unwrap_or_default(),
        });
    ports.unit_of_work.commit(writes).await
}
