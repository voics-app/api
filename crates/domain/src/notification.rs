//! Per-user notification settings for a guild (or for DMs) and private user notes.
//!
//! Discord semantics: a channel override beats the guild level, which beats the guild's
//! `default_message_notifications`; a mute without an end time lasts until it is turned
//! off; a mute whose end time has passed is ignored; a muted guild mutes all its channels.

use chrono::{DateTime, Duration, Utc};

use crate::validation::FieldError;
use crate::Snowflake;

/// Settings of the user's direct messages are stored under this guild id (Discord's `@me`).
pub const DIRECT_MESSAGES: Snowflake = Snowflake::new(0);
/// Upper bound on channel overrides per guild.
pub const MAX_CHANNEL_OVERRIDES: usize = 1000;
/// Upper bound on a note, in characters.
pub const MAX_NOTE_CHARS: usize = 256;
/// Longest accepted `selected_time_window`, in seconds.
pub const MAX_MUTE_WINDOW_SECS: i64 = 365 * 24 * 3600;

fn invalid(field: &str, code: &'static str, message: &str) -> FieldError {
    FieldError {
        field: field.to_owned(),
        code,
        message: message.to_owned(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyLevel {
    AllMessages,
    OnlyMentions,
    NoMessages,
}

impl NotifyLevel {
    pub const fn code(self) -> u8 {
        match self {
            NotifyLevel::AllMessages => 0,
            NotifyLevel::OnlyMentions => 1,
            NotifyLevel::NoMessages => 2,
        }
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(NotifyLevel::AllMessages),
            1 => Some(NotifyLevel::OnlyMentions),
            2 => Some(NotifyLevel::NoMessages),
            _ => None,
        }
    }

    /// Parses a wire value: 0..=2, with `None` and Discord's `3` meaning "inherit".
    pub fn parse_optional(field: &str, raw: Option<u8>) -> Result<Option<Self>, FieldError> {
        match raw {
            None | Some(3) => Ok(None),
            Some(code) => Self::from_code(code).map(Some).ok_or_else(|| {
                invalid(
                    field,
                    "BASE_TYPE_CHOICES",
                    "Must be 0 (all), 1 (mentions), 2 (nothing) or null.",
                )
            }),
        }
    }
}

/// A mute. `end_time: None` while `muted` means until turned off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mute {
    pub muted: bool,
    pub end_time: Option<DateTime<Utc>>,
}

/// The `mute_config` a client sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MuteConfigInput {
    pub end_time: Option<DateTime<Utc>>,
    /// Seconds from now; `-1` or absent with no `end_time` means indefinitely.
    pub selected_time_window: Option<i64>,
}

impl Mute {
    pub const OFF: Mute = Mute {
        muted: false,
        end_time: None,
    };

    /// Whether the mute applies at `now` (an expired mute does not).
    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        self.muted && self.end_time.is_none_or(|end| end > now)
    }

    /// Applies an update: `muted` and `config` are what the caller sent (both optional).
    pub fn update(
        self,
        field: &str,
        muted: Option<bool>,
        config: Option<Option<MuteConfigInput>>,
        now: DateTime<Utc>,
    ) -> Result<Mute, FieldError> {
        let muted_now = muted.unwrap_or(self.muted);
        if !muted_now {
            return Ok(Mute::OFF);
        }
        match config {
            Some(Some(config)) => {
                let end_time = match (config.end_time, config.selected_time_window) {
                    (Some(end), _) => Some(end),
                    (None, Some(window)) if window > 0 => {
                        if window > MAX_MUTE_WINDOW_SECS {
                            return Err(invalid(
                                field,
                                "BASE_TYPE_BAD_LENGTH",
                                "selected_time_window is too long.",
                            ));
                        }
                        Some(now + Duration::seconds(window))
                    }
                    (None, Some(window)) if window != -1 => {
                        return Err(invalid(
                            field,
                            "BASE_TYPE_CHOICES",
                            "selected_time_window must be positive or -1.",
                        ));
                    }
                    (None, _) => None,
                };
                Ok(Mute {
                    muted: true,
                    end_time,
                })
            }
            Some(None) => Ok(Mute {
                muted: true,
                end_time: None,
            }),
            // Re-muting without a config mutes indefinitely; otherwise keep the current one.
            None if muted == Some(true) => Ok(Mute {
                muted: true,
                end_time: None,
            }),
            None => Ok(self),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelOverride {
    pub channel_id: Snowflake,
    /// `None` inherits the guild level.
    pub level: Option<NotifyLevel>,
    pub mute: Mute,
}

impl ChannelOverride {
    /// An override that changes nothing is dropped instead of stored.
    pub fn is_default(&self) -> bool {
        self.level.is_none() && !self.mute.muted
    }
}

/// One channel override as the client sends it: fields it leaves out keep their stored value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelOverrideInput {
    pub channel_id: Snowflake,
    /// Absent keeps the stored level, `Some(None)` (null) inherits the guild level.
    pub level: Option<Option<NotifyLevel>>,
    pub muted: Option<bool>,
    pub mute_config: Option<Option<MuteConfigInput>>,
}

/// A partial update of [`GuildNotificationSettings`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuildNotificationPatch {
    pub level: Option<Option<NotifyLevel>>,
    pub suppress_everyone: Option<bool>,
    pub suppress_roles: Option<bool>,
    pub muted: Option<bool>,
    pub mute_config: Option<Option<MuteConfigInput>>,
    pub hide_muted_channels: Option<bool>,
    pub channel_overrides: Option<Vec<ChannelOverrideInput>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildNotificationSettings {
    pub user_id: Snowflake,
    /// [`DIRECT_MESSAGES`] for the user's DMs.
    pub guild_id: Snowflake,
    /// `None` uses the guild's `default_message_notifications`.
    pub level: Option<NotifyLevel>,
    pub suppress_everyone: bool,
    pub suppress_roles: bool,
    pub mute: Mute,
    pub hide_muted_channels: bool,
    pub channel_overrides: Vec<ChannelOverride>,
}

impl GuildNotificationSettings {
    pub fn defaults_for(user_id: Snowflake, guild_id: Snowflake) -> Self {
        Self {
            user_id,
            guild_id,
            level: None,
            suppress_everyone: false,
            suppress_roles: false,
            mute: Mute::OFF,
            hide_muted_channels: false,
            channel_overrides: Vec::new(),
        }
    }

    pub fn is_direct_messages(&self) -> bool {
        self.guild_id == DIRECT_MESSAGES
    }

    pub fn channel_override(&self, channel_id: Snowflake) -> Option<&ChannelOverride> {
        self.channel_overrides
            .iter()
            .find(|o| o.channel_id == channel_id)
    }

    /// Channel override, else guild level, else the guild default.
    pub fn effective_level(
        &self,
        channel_id: Snowflake,
        guild_default: NotifyLevel,
    ) -> NotifyLevel {
        self.channel_override(channel_id)
            .and_then(|o| o.level)
            .or(self.level)
            .unwrap_or(guild_default)
    }

    /// Whether the channel is muted at `now`, by its own mute or the guild's.
    pub fn is_channel_muted(&self, channel_id: Snowflake, now: DateTime<Utc>) -> bool {
        self.mute.is_active(now)
            || self
                .channel_override(channel_id)
                .is_some_and(|o| o.mute.is_active(now))
    }

    /// Applies `patch`. Override entries must already have been checked to belong to the guild.
    pub fn apply(
        &mut self,
        patch: GuildNotificationPatch,
        now: DateTime<Utc>,
    ) -> Result<(), FieldError> {
        if let Some(level) = patch.level {
            self.level = level;
        }
        if let Some(v) = patch.suppress_everyone {
            self.suppress_everyone = v;
        }
        if let Some(v) = patch.suppress_roles {
            self.suppress_roles = v;
        }
        if let Some(v) = patch.hide_muted_channels {
            self.hide_muted_channels = v;
        }
        if patch.muted.is_some() || patch.mute_config.is_some() {
            self.mute = self
                .mute
                .update("mute_config", patch.muted, patch.mute_config, now)?;
        }
        if let Some(entries) = patch.channel_overrides {
            for entry in entries {
                let stored = self.channel_override(entry.channel_id).copied();
                let current = stored.map_or(Mute::OFF, |o| o.mute);
                let mute =
                    current.update("channel_overrides", entry.muted, entry.mute_config, now)?;
                let level = entry.level.unwrap_or(stored.and_then(|o| o.level));
                self.channel_overrides
                    .retain(|o| o.channel_id != entry.channel_id);
                let next = ChannelOverride {
                    channel_id: entry.channel_id,
                    level,
                    mute,
                };
                if !next.is_default() {
                    self.channel_overrides.push(next);
                }
            }
            if self.channel_overrides.len() > MAX_CHANNEL_OVERRIDES {
                return Err(invalid(
                    "channel_overrides",
                    "BASE_TYPE_BAD_LENGTH",
                    "Too many channel overrides.",
                ));
            }
            self.channel_overrides.sort_by_key(|o| o.channel_id);
        }
        Ok(())
    }
}

/// A note is stored as written; an empty note means "no note".
pub fn validate_note(note: &str) -> Result<(), FieldError> {
    if note.chars().count() > MAX_NOTE_CHARS {
        return Err(invalid(
            "note",
            "BASE_TYPE_BAD_LENGTH",
            "Must be 256 or fewer in length.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sf(n: u64) -> Snowflake {
        Snowflake::new(n)
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-20T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn settings() -> GuildNotificationSettings {
        GuildNotificationSettings::defaults_for(sf(1), sf(10))
    }

    fn entry(channel: u64, level: Option<NotifyLevel>, muted: bool) -> ChannelOverrideInput {
        ChannelOverrideInput {
            channel_id: sf(channel),
            level: Some(level),
            muted: Some(muted),
            mute_config: None,
        }
    }

    fn window(secs: i64) -> MuteConfigInput {
        MuteConfigInput {
            end_time: None,
            selected_time_window: Some(secs),
        }
    }

    #[test]
    fn level_codes_and_inherit() {
        assert_eq!(
            NotifyLevel::parse_optional("f", Some(1)).unwrap(),
            Some(NotifyLevel::OnlyMentions)
        );
        assert_eq!(NotifyLevel::parse_optional("f", Some(3)).unwrap(), None);
        assert_eq!(NotifyLevel::parse_optional("f", None).unwrap(), None);
        assert!(NotifyLevel::parse_optional("f", Some(4)).is_err());
    }

    #[test]
    fn channel_override_beats_guild_beats_default() {
        let mut s = settings();
        let all = NotifyLevel::AllMessages;
        assert_eq!(s.effective_level(sf(20), all), all);
        s.level = Some(NotifyLevel::OnlyMentions);
        assert_eq!(s.effective_level(sf(20), all), NotifyLevel::OnlyMentions);
        s.apply(
            GuildNotificationPatch {
                channel_overrides: Some(vec![entry(20, Some(NotifyLevel::NoMessages), false)]),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        assert_eq!(s.effective_level(sf(20), all), NotifyLevel::NoMessages);
        assert_eq!(s.effective_level(sf(21), all), NotifyLevel::OnlyMentions);
    }

    #[test]
    fn indefinite_mute_has_no_end_time() {
        let mute = Mute::OFF.update("m", Some(true), None, now()).unwrap();
        assert!(mute.muted && mute.end_time.is_none());
        assert!(mute.is_active(now() + Duration::days(3650)));
    }

    #[test]
    fn mute_window_sets_end_time_and_expires() {
        let mute = Mute::OFF
            .update("m", Some(true), Some(Some(window(900))), now())
            .unwrap();
        assert_eq!(mute.end_time, Some(now() + Duration::minutes(15)));
        assert!(mute.is_active(now() + Duration::minutes(14)));
        assert!(!mute.is_active(now() + Duration::minutes(15)));
        assert!(!mute.is_active(now() + Duration::hours(2)));
    }

    #[test]
    fn explicit_end_time_wins_and_minus_one_is_forever() {
        let end = now() + Duration::hours(1);
        let cfg = MuteConfigInput {
            end_time: Some(end),
            selected_time_window: Some(60),
        };
        let mute = Mute::OFF
            .update("m", Some(true), Some(Some(cfg)), now())
            .unwrap();
        assert_eq!(mute.end_time, Some(end));
        let mute = Mute::OFF
            .update("m", Some(true), Some(Some(window(-1))), now())
            .unwrap();
        assert_eq!(mute.end_time, None);
    }

    #[test]
    fn rejects_bad_windows() {
        for secs in [0, -2, MAX_MUTE_WINDOW_SECS + 1] {
            let result = Mute::OFF.update("m", Some(true), Some(Some(window(secs))), now());
            assert!(result.is_err(), "{secs}");
        }
    }

    #[test]
    fn unmuting_clears_end_time() {
        let muted = Mute {
            muted: true,
            end_time: Some(now() + Duration::hours(1)),
        };
        assert_eq!(
            muted.update("m", Some(false), None, now()).unwrap(),
            Mute::OFF
        );
    }

    #[test]
    fn config_alone_keeps_current_muted_state() {
        let cfg = MuteConfigInput {
            end_time: Some(now() + Duration::hours(1)),
            selected_time_window: None,
        };
        let unmuted = Mute::OFF.update("m", None, Some(Some(cfg)), now()).unwrap();
        assert_eq!(unmuted, Mute::OFF);
        let muted = Mute {
            muted: true,
            end_time: None,
        };
        let updated = muted.update("m", None, Some(Some(cfg)), now()).unwrap();
        assert_eq!(updated.end_time, cfg.end_time);
    }

    #[test]
    fn guild_mute_mutes_every_channel_until_it_expires() {
        let mut s = settings();
        s.apply(
            GuildNotificationPatch {
                muted: Some(true),
                mute_config: Some(Some(window(3600))),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        assert!(s.is_channel_muted(sf(99), now() + Duration::minutes(59)));
        assert!(!s.is_channel_muted(sf(99), now() + Duration::minutes(61)));
    }

    #[test]
    fn channel_mute_is_local_and_expired_channel_mute_is_ignored() {
        let mut s = settings();
        let mut e = entry(20, None, true);
        e.mute_config = Some(Some(window(60)));
        s.apply(
            GuildNotificationPatch {
                channel_overrides: Some(vec![e]),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        assert!(s.is_channel_muted(sf(20), now()));
        assert!(!s.is_channel_muted(sf(21), now()));
        assert!(!s.is_channel_muted(sf(20), now() + Duration::minutes(2)));
    }

    #[test]
    fn default_override_entries_are_removed_and_entries_replace() {
        let mut s = settings();
        s.apply(
            GuildNotificationPatch {
                channel_overrides: Some(vec![entry(20, Some(NotifyLevel::OnlyMentions), true)]),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        assert_eq!(s.channel_overrides.len(), 1);
        s.apply(
            GuildNotificationPatch {
                channel_overrides: Some(vec![entry(20, None, false)]),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        assert!(s.channel_overrides.is_empty());
    }

    #[test]
    fn omitted_override_fields_keep_their_stored_value() {
        let mut s = settings();
        s.apply(
            GuildNotificationPatch {
                channel_overrides: Some(vec![entry(20, Some(NotifyLevel::OnlyMentions), false)]),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        let mute_only = ChannelOverrideInput {
            channel_id: sf(20),
            level: None,
            muted: Some(true),
            mute_config: None,
        };
        s.apply(
            GuildNotificationPatch {
                channel_overrides: Some(vec![mute_only]),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        let o = s.channel_override(sf(20)).unwrap();
        assert_eq!(o.level, Some(NotifyLevel::OnlyMentions));
        assert!(o.mute.muted);
    }

    #[test]
    fn override_count_is_capped() {
        let mut s = settings();
        let entries = (1..=MAX_CHANNEL_OVERRIDES as u64 + 1)
            .map(|n| entry(n, Some(NotifyLevel::NoMessages), false))
            .collect();
        let result = s.apply(
            GuildNotificationPatch {
                channel_overrides: Some(entries),
                ..Default::default()
            },
            now(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn patch_changes_only_what_it_names() {
        let mut s = settings();
        s.level = Some(NotifyLevel::OnlyMentions);
        s.apply(
            GuildNotificationPatch {
                suppress_everyone: Some(true),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        assert!(s.suppress_everyone && !s.suppress_roles);
        assert_eq!(s.level, Some(NotifyLevel::OnlyMentions));
        s.apply(
            GuildNotificationPatch {
                level: Some(None),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        assert_eq!(s.level, None);
    }

    #[test]
    fn note_limit_counts_characters() {
        assert!(validate_note(&"\u{e9}".repeat(256)).is_ok());
        assert!(validate_note(&"\u{e9}".repeat(257)).is_err());
        assert!(validate_note("").is_ok());
    }
}
