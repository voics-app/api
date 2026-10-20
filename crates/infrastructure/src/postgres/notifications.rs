//! Storage mapping for notification settings (`user_guild_settings`) and notes (`user_notes`).

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::FromRow;

use domain::notification::{ChannelOverride, GuildNotificationSettings, Mute, NotifyLevel};
use domain::Snowflake;

use super::convert::sf;

pub(super) const COLUMNS: &str = "user_id, guild_id, message_notifications, suppress_everyone, \
     suppress_roles, muted, mute_end_time, hide_muted_channels, channel_overrides";

#[derive(FromRow)]
pub(super) struct Row {
    user_id: i64,
    guild_id: i64,
    message_notifications: Option<i16>,
    suppress_everyone: bool,
    suppress_roles: bool,
    muted: bool,
    mute_end_time: Option<DateTime<Utc>>,
    hide_muted_channels: bool,
    channel_overrides: Value,
}

fn level(raw: Option<i64>) -> Option<NotifyLevel> {
    raw.and_then(|code| u8::try_from(code).ok())
        .and_then(NotifyLevel::from_code)
}

fn override_from_json(value: &Value) -> Option<ChannelOverride> {
    let channel_id = value
        .get("channel_id")?
        .as_str()?
        .parse::<Snowflake>()
        .ok()?;
    let muted = value.get("muted").and_then(Value::as_bool).unwrap_or(false);
    let end_time = value
        .get("mute_end_time")
        .and_then(Value::as_str)
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|t| t.with_timezone(&Utc));
    Some(ChannelOverride {
        channel_id,
        level: level(value.get("message_notifications").and_then(Value::as_i64)),
        mute: Mute { muted, end_time },
    })
}

pub(super) fn overrides_json(overrides: &[ChannelOverride]) -> Value {
    Value::Array(
        overrides
            .iter()
            .map(|o| {
                json!({
                    "channel_id": o.channel_id.to_string(),
                    "message_notifications": o.level.map(|l| l.code()),
                    "muted": o.mute.muted,
                    "mute_end_time": o.mute.end_time.map(|t| t.to_rfc3339()),
                })
            })
            .collect(),
    )
}

impl From<Row> for GuildNotificationSettings {
    fn from(row: Row) -> Self {
        GuildNotificationSettings {
            user_id: sf(row.user_id),
            guild_id: sf(row.guild_id),
            level: level(row.message_notifications.map(i64::from)),
            suppress_everyone: row.suppress_everyone,
            suppress_roles: row.suppress_roles,
            mute: Mute {
                muted: row.muted,
                end_time: row.mute_end_time,
            },
            hide_muted_channels: row.hide_muted_channels,
            channel_overrides: row
                .channel_overrides
                .as_array()
                .map(|items| items.iter().filter_map(override_from_json).collect())
                .unwrap_or_default(),
        }
    }
}
