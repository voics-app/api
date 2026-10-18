use domain::relationship::{RelationshipChange, RelationshipType};
use domain::user::User;

use crate::context::Ports;
use crate::events::DomainEvent;
use crate::write_set::{Mutation, WriteSet};

/// Current relationship kinds in both directions.
pub async fn current_pair(
    ports: &Ports,
    me: &User,
    them: &User,
) -> crate::error::AppResult<(Option<RelationshipType>, Option<RelationshipType>)> {
    let mine = ports
        .relationships
        .get(me.id, them.id)
        .await?
        .map(|r| r.kind);
    let theirs = ports
        .relationships
        .get(them.id, me.id)
        .await?
        .map(|r| r.kind);
    Ok((mine, theirs))
}

/// Writes a relationship change for both sides and emits the matching events.
pub fn apply_change(
    ports: &Ports,
    writes: &mut WriteSet,
    me: &User,
    them: &User,
    before: (Option<RelationshipType>, Option<RelationshipType>),
    change: RelationshipChange,
) {
    let since = ports.clock.now();
    apply_side(writes, me, them, before.0, change.mine, since);
    apply_side(writes, them, me, before.1, change.theirs, since);
}

fn apply_side(
    writes: &mut WriteSet,
    owner: &User,
    target: &User,
    before: Option<RelationshipType>,
    after: Option<RelationshipType>,
    since: chrono::DateTime<chrono::Utc>,
) {
    if before == after {
        return;
    }
    writes.mutate(Mutation::SetRelationship {
        user_id: owner.id,
        target_id: target.id,
        kind: after,
        since,
    });
    match after {
        Some(kind) => {
            writes.emit(DomainEvent::RelationshipAdd {
                user_id: owner.id,
                kind,
                target: target.clone(),
                since,
            });
        }
        None => {
            if let Some(kind) = before {
                writes.emit(DomainEvent::RelationshipRemove {
                    user_id: owner.id,
                    target_id: target.id,
                    kind,
                });
            }
        }
    }
}
