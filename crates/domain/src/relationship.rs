use chrono::{DateTime, Utc};

use crate::snowflake::Snowflake;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelationshipType {
    Friend,
    Blocked,
    IncomingRequest,
    OutgoingRequest,
}

impl RelationshipType {
    pub const fn code(self) -> u8 {
        match self {
            RelationshipType::Friend => 1,
            RelationshipType::Blocked => 2,
            RelationshipType::IncomingRequest => 3,
            RelationshipType::OutgoingRequest => 4,
        }
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => RelationshipType::Friend,
            2 => RelationshipType::Blocked,
            3 => RelationshipType::IncomingRequest,
            4 => RelationshipType::OutgoingRequest,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    pub user_id: Snowflake,
    pub target_id: Snowflake,
    pub kind: RelationshipType,
    pub since: DateTime<Utc>,
}

/// Desired state of both sides after an action. `None` means "remove".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelationshipChange {
    pub mine: Option<RelationshipType>,
    pub theirs: Option<RelationshipType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RelationshipError {
    #[error("you cannot add yourself")]
    SelfTarget,
    #[error("this user is not accepting friend requests from you")]
    BlockedByTarget,
    #[error("bots cannot have relationships")]
    BotTarget,
}

/// Sending (or accepting) a friend request.
pub fn send_friend_request(
    mine: Option<RelationshipType>,
    theirs: Option<RelationshipType>,
) -> Result<RelationshipChange, RelationshipError> {
    use RelationshipType::*;
    if theirs == Some(Blocked) {
        return Err(RelationshipError::BlockedByTarget);
    }
    Ok(match mine {
        Some(Friend) => RelationshipChange { mine: Some(Friend), theirs: Some(Friend) },
        Some(IncomingRequest) => RelationshipChange { mine: Some(Friend), theirs: Some(Friend) },
        Some(OutgoingRequest) | Some(Blocked) | None => {
            RelationshipChange { mine: Some(OutgoingRequest), theirs: Some(IncomingRequest) }
        }
    })
}

/// Blocking removes any friendship or pending request from the other side,
/// but keeps the other side's own block.
pub fn block(theirs: Option<RelationshipType>) -> RelationshipChange {
    let theirs = match theirs {
        Some(RelationshipType::Blocked) => Some(RelationshipType::Blocked),
        _ => None,
    };
    RelationshipChange { mine: Some(RelationshipType::Blocked), theirs }
}

/// Removing a friend, cancelling/declining a request or unblocking.
pub fn remove(mine: Option<RelationshipType>, theirs: Option<RelationshipType>) -> RelationshipChange {
    let theirs = match (mine, theirs) {
        (_, Some(RelationshipType::Blocked)) => Some(RelationshipType::Blocked),
        (Some(RelationshipType::Blocked), other) => other,
        _ => None,
    };
    RelationshipChange { mine: None, theirs }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RelationshipType::*;

    fn change(mine: Option<RelationshipType>, theirs: Option<RelationshipType>) -> RelationshipChange {
        RelationshipChange { mine, theirs }
    }

    #[test]
    fn friend_request_table() {
        let cases = [
            ("fresh request", None, None, Ok(change(Some(OutgoingRequest), Some(IncomingRequest)))),
            ("accepting their request", Some(IncomingRequest), Some(OutgoingRequest), Ok(change(Some(Friend), Some(Friend)))),
            ("repeat request is idempotent", Some(OutgoingRequest), Some(IncomingRequest), Ok(change(Some(OutgoingRequest), Some(IncomingRequest)))),
            ("already friends", Some(Friend), Some(Friend), Ok(change(Some(Friend), Some(Friend)))),
            ("request after I blocked them unblocks", Some(Blocked), None, Ok(change(Some(OutgoingRequest), Some(IncomingRequest)))),
            ("they blocked me", None, Some(Blocked), Err(RelationshipError::BlockedByTarget)),
        ];
        for (name, mine, theirs, expected) in cases {
            assert_eq!(send_friend_request(mine, theirs), expected, "{name}");
        }
    }

    #[test]
    fn block_table() {
        assert_eq!(block(Some(Friend)), change(Some(Blocked), None));
        assert_eq!(block(Some(IncomingRequest)), change(Some(Blocked), None));
        assert_eq!(block(Some(Blocked)), change(Some(Blocked), Some(Blocked)));
        assert_eq!(block(None), change(Some(Blocked), None));
    }

    #[test]
    fn remove_table() {
        assert_eq!(remove(Some(Friend), Some(Friend)), change(None, None));
        assert_eq!(remove(Some(OutgoingRequest), Some(IncomingRequest)), change(None, None));
        assert_eq!(remove(Some(Blocked), Some(Blocked)), change(None, Some(Blocked)));
        assert_eq!(remove(Some(Friend), Some(Blocked)), change(None, Some(Blocked)));
        assert_eq!(remove(Some(Blocked), None), change(None, None));
    }

    #[test]
    fn codes_round_trip() {
        for code in 1..=4 {
            assert_eq!(RelationshipType::from_code(code).unwrap().code(), code);
        }
        assert!(RelationshipType::from_code(0).is_none());
    }
}
