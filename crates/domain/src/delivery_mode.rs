//! Delivery-mode resolution (pinned interface P11) — the single
//! implementation used by the whole platform.

use std::fmt;
use std::str::FromStr;

/// Ordered by strictness: `Standard < Ephemeral < P2p`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum DeliveryMode {
    #[default]
    Standard,
    Ephemeral,
    P2p,
}

/// Ephemeral is only allowed in guild channels visible to at most this many members.
pub const EPHEMERAL_MAX_GUILD_VIEWERS: usize = 100;
/// P2P is only allowed in private channels with at most this many participants.
pub const P2P_MAX_PARTICIPANTS: usize = 10;

impl DeliveryMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            DeliveryMode::Standard => "standard",
            DeliveryMode::Ephemeral => "ephemeral",
            DeliveryMode::P2p => "p2p",
        }
    }

    pub const fn strictness(self) -> u8 {
        match self {
            DeliveryMode::Standard => 0,
            DeliveryMode::Ephemeral => 1,
            DeliveryMode::P2p => 2,
        }
    }
}

impl fmt::Display for DeliveryMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DeliveryMode {
    type Err = DeliveryModeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "standard" => Ok(DeliveryMode::Standard),
            "ephemeral" => Ok(DeliveryMode::Ephemeral),
            "p2p" => Ok(DeliveryMode::P2p),
            _ => Err(DeliveryModeError::Unknown),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DeliveryModeError {
    #[error("unknown delivery mode")]
    Unknown,
    #[error("p2p delivery is not available in guild channels")]
    P2pInGuild,
    #[error("ephemeral delivery is limited to channels visible to at most 100 members")]
    EphemeralAudienceTooLarge,
    #[error("p2p delivery is limited to chats with at most 10 members")]
    P2pAudienceTooLarge,
}

/// One participant's preference in a DM or group DM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParticipantPreference {
    /// Per-chat override.
    pub chat_override: Option<DeliveryMode>,
    /// Account-wide default.
    pub account_default: DeliveryMode,
}

impl ParticipantPreference {
    fn effective(self) -> DeliveryMode {
        self.chat_override.unwrap_or(self.account_default)
    }
}

/// What the resolver needs to know about a channel.
#[derive(Debug, Clone, Copy)]
pub enum ChannelDelivery<'a> {
    /// Guild channel: the mode chosen by members holding MANAGE_CHANNELS.
    Guild { configured: DeliveryMode },
    /// DM or group DM: every participant's preference.
    Private {
        participants: &'a [ParticipantPreference],
    },
}

/// `effective(channel)` from P11.
pub fn resolve(channel: ChannelDelivery<'_>) -> DeliveryMode {
    match channel {
        ChannelDelivery::Guild { configured } => configured,
        ChannelDelivery::Private { participants } => {
            let p2p_allowed = participants.len() <= P2P_MAX_PARTICIPANTS;
            participants
                .iter()
                .map(|p| p.effective())
                .map(|mode| match mode {
                    DeliveryMode::P2p if !p2p_allowed => DeliveryMode::Ephemeral,
                    other => other,
                })
                .max()
                .unwrap_or_default()
        }
    }
}

/// Validates a mode chosen for a guild channel (400 on violation).
pub fn validate_guild_channel_mode(
    mode: DeliveryMode,
    viewer_count: usize,
) -> Result<(), DeliveryModeError> {
    match mode {
        DeliveryMode::Standard => Ok(()),
        DeliveryMode::P2p => Err(DeliveryModeError::P2pInGuild),
        DeliveryMode::Ephemeral if viewer_count > EPHEMERAL_MAX_GUILD_VIEWERS => {
            Err(DeliveryModeError::EphemeralAudienceTooLarge)
        }
        DeliveryMode::Ephemeral => Ok(()),
    }
}

/// Validates a per-chat override in a DM or group DM (400 on violation).
pub fn validate_private_override(
    mode: DeliveryMode,
    participant_count: usize,
) -> Result<(), DeliveryModeError> {
    if mode == DeliveryMode::P2p && participant_count > P2P_MAX_PARTICIPANTS {
        return Err(DeliveryModeError::P2pAudienceTooLarge);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use DeliveryMode::{Ephemeral as E, P2p as P, Standard as S};

    fn pref(
        chat_override: Option<DeliveryMode>,
        account_default: DeliveryMode,
    ) -> ParticipantPreference {
        ParticipantPreference {
            chat_override,
            account_default,
        }
    }

    #[test]
    fn strictness_order() {
        assert!(S < E && E < P);
        assert_eq!([S.strictness(), E.strictness(), P.strictness()], [0, 1, 2]);
    }

    #[test]
    fn guild_channels_use_the_configured_mode() {
        for mode in [S, E, P] {
            assert_eq!(resolve(ChannelDelivery::Guild { configured: mode }), mode);
        }
    }

    #[test]
    fn private_channel_resolution_table() {
        let table: Vec<(&str, Vec<ParticipantPreference>, DeliveryMode)> = vec![
            (
                "both standard defaults",
                vec![pref(None, S), pref(None, S)],
                S,
            ),
            (
                "one ephemeral default wins",
                vec![pref(None, S), pref(None, E)],
                E,
            ),
            (
                "one p2p default wins",
                vec![pref(None, E), pref(None, P)],
                P,
            ),
            (
                "override beats own default (downgrade)",
                vec![pref(Some(S), P), pref(None, S)],
                S,
            ),
            (
                "override beats own default (upgrade)",
                vec![pref(Some(E), S), pref(None, S)],
                E,
            ),
            (
                "strictest override wins",
                vec![pref(Some(P), S), pref(Some(E), S)],
                P,
            ),
            (
                "other's default still counts",
                vec![pref(Some(S), S), pref(None, E)],
                E,
            ),
            (
                "all overrides standard",
                vec![pref(Some(S), P), pref(Some(S), E)],
                S,
            ),
            ("no participants defaults to standard", vec![], S),
            (
                "ten participants keep p2p",
                (0..10)
                    .map(|i| pref(None, if i == 0 { P } else { S }))
                    .collect(),
                P,
            ),
            (
                "eleven participants: p2p falls back to ephemeral",
                (0..11)
                    .map(|i| pref(None, if i == 0 { P } else { S }))
                    .collect(),
                E,
            ),
            (
                "eleven participants: p2p override falls back to ephemeral",
                (0..11)
                    .map(|i| pref(if i == 3 { Some(P) } else { None }, S))
                    .collect(),
                E,
            ),
            (
                "group of three, mixed",
                vec![pref(None, S), pref(Some(E), P), pref(None, S)],
                E,
            ),
        ];
        for (name, participants, expected) in table {
            assert_eq!(
                resolve(ChannelDelivery::Private {
                    participants: &participants
                }),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn guild_channel_validation_table() {
        let table = [
            (S, 10_000, Ok(())),
            (E, 1, Ok(())),
            (E, 100, Ok(())),
            (E, 101, Err(DeliveryModeError::EphemeralAudienceTooLarge)),
            (P, 2, Err(DeliveryModeError::P2pInGuild)),
            (P, 500, Err(DeliveryModeError::P2pInGuild)),
        ];
        for (mode, viewers, expected) in table {
            assert_eq!(
                validate_guild_channel_mode(mode, viewers),
                expected,
                "{mode} with {viewers} viewers"
            );
        }
    }

    #[test]
    fn private_override_validation_table() {
        let table = [
            (S, 25, Ok(())),
            (E, 25, Ok(())),
            (P, 2, Ok(())),
            (P, 10, Ok(())),
            (P, 11, Err(DeliveryModeError::P2pAudienceTooLarge)),
        ];
        for (mode, participants, expected) in table {
            assert_eq!(
                validate_private_override(mode, participants),
                expected,
                "{mode} with {participants}"
            );
        }
    }

    #[test]
    fn parses_and_prints_wire_names() {
        for mode in [S, E, P] {
            assert_eq!(mode.as_str().parse::<DeliveryMode>().unwrap(), mode);
        }
        assert_eq!(
            "secret".parse::<DeliveryMode>(),
            Err(DeliveryModeError::Unknown)
        );
    }
}
