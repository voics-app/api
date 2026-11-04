//! Personal pins synced across devices (client#222, client#223): pinned DMs and pinned channels per server.
//! Both lists keep the most recently pinned entry last and never hold duplicates.

use std::collections::{BTreeMap, HashSet};

use crate::snowflake::Snowflake;

pub const MAX_PINNED_DMS: usize = 5;
pub const MAX_PINNED_PER_GUILD: usize = 10;
pub const MAX_PINNED_GUILDS: usize = 200;

/// Pinned channels keyed by server id.
pub type PinnedChannels = BTreeMap<Snowflake, Vec<Snowflake>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinError {
    TooManyDms,
    TooManyChannels,
    TooManyGuilds,
    Duplicate,
}

impl PinError {
    pub fn message(self) -> &'static str {
        match self {
            PinError::TooManyDms => "At most 5 DMs can be pinned.",
            PinError::TooManyChannels => "At most 10 channels can be pinned per server.",
            PinError::TooManyGuilds => "Too many servers.",
            PinError::Duplicate => "A channel can only be pinned once.",
        }
    }
}

fn has_duplicates(ids: &[Snowflake]) -> bool {
    let mut seen = HashSet::new();
    ids.iter().any(|id| !seen.insert(*id))
}

pub fn validate_pinned_dms(ids: &[Snowflake]) -> Result<(), PinError> {
    if ids.len() > MAX_PINNED_DMS {
        return Err(PinError::TooManyDms);
    }
    if has_duplicates(ids) {
        return Err(PinError::Duplicate);
    }
    Ok(())
}

/// Validates the map and drops servers without pins, returning the normalised map.
pub fn normalize_pinned_channels(pinned: PinnedChannels) -> Result<PinnedChannels, PinError> {
    let pinned: PinnedChannels = pinned
        .into_iter()
        .filter(|(_, channels)| !channels.is_empty())
        .collect();
    if pinned.len() > MAX_PINNED_GUILDS {
        return Err(PinError::TooManyGuilds);
    }
    for channels in pinned.values() {
        if channels.len() > MAX_PINNED_PER_GUILD {
            return Err(PinError::TooManyChannels);
        }
        if has_duplicates(channels) {
            return Err(PinError::Duplicate);
        }
    }
    Ok(pinned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(range: std::ops::Range<u64>) -> Vec<Snowflake> {
        range.map(Snowflake::new).collect()
    }

    #[test]
    fn five_dms_are_allowed_and_six_are_not() {
        assert_eq!(validate_pinned_dms(&ids(1..6)), Ok(()));
        assert_eq!(validate_pinned_dms(&ids(1..7)), Err(PinError::TooManyDms));
    }

    #[test]
    fn duplicate_dms_are_rejected() {
        let dup = vec![Snowflake::new(1), Snowflake::new(1)];
        assert_eq!(validate_pinned_dms(&dup), Err(PinError::Duplicate));
    }

    #[test]
    fn empty_dm_list_is_valid() {
        assert_eq!(validate_pinned_dms(&[]), Ok(()));
    }

    #[test]
    fn ten_channels_per_guild_are_allowed_and_eleven_are_not() {
        let ok = PinnedChannels::from([(Snowflake::new(100), ids(1..11))]);
        assert_eq!(normalize_pinned_channels(ok.clone()), Ok(ok));
        let bad = PinnedChannels::from([(Snowflake::new(100), ids(1..12))]);
        assert_eq!(
            normalize_pinned_channels(bad),
            Err(PinError::TooManyChannels)
        );
    }

    #[test]
    fn duplicate_channels_in_a_guild_are_rejected() {
        let dup = PinnedChannels::from([(Snowflake::new(100), vec![Snowflake::new(1); 2])]);
        assert_eq!(normalize_pinned_channels(dup), Err(PinError::Duplicate));
    }

    #[test]
    fn the_same_channel_id_in_two_guilds_is_fine() {
        let ok = PinnedChannels::from([
            (Snowflake::new(100), vec![Snowflake::new(1)]),
            (Snowflake::new(200), vec![Snowflake::new(1)]),
        ]);
        assert_eq!(normalize_pinned_channels(ok.clone()), Ok(ok));
    }

    #[test]
    fn guilds_without_pins_are_dropped() {
        let input = PinnedChannels::from([
            (Snowflake::new(100), Vec::new()),
            (Snowflake::new(200), vec![Snowflake::new(1)]),
        ]);
        let expected = PinnedChannels::from([(Snowflake::new(200), vec![Snowflake::new(1)])]);
        assert_eq!(normalize_pinned_channels(input), Ok(expected));
    }

    #[test]
    fn too_many_guilds_are_rejected() {
        let many: PinnedChannels = (1..=201)
            .map(|g| (Snowflake::new(g), vec![Snowflake::new(1)]))
            .collect();
        assert_eq!(
            normalize_pinned_channels(many),
            Err(PinError::TooManyGuilds)
        );
    }
}
