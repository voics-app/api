use crate::{
    context::{Actor, Ports},
    error::{AppError, AppResult},
    support::guild_context::GuildContext,
};
use chrono::{Duration, NaiveDate};
use domain::{permissions::Permissions, Snowflake};
use futures::{stream, StreamExt};

#[derive(Debug)]
pub struct Day {
    pub day: NaiveDate,
    pub joins: Option<i64>,
    pub leaves: Option<i64>,
    pub membership_complete: bool,
    pub membership_coverage: &'static str,
    pub messages: Option<u64>,
    pub message_complete: bool,
    pub message_coverage: &'static str,
    pub message_source_initialized_at_ms: Option<u64>,
    pub activity_source_initialized_at_ms: i64,
    pub voice_minutes: Option<f64>,
    pub voice_coverage: &'static str,
}

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    days: u32,
) -> AppResult<Vec<Day>> {
    if days != 7 && days != 30 {
        return Err(AppError::invalid(
            "period",
            "BASE_TYPE_CHOICES",
            "Period must be 7 or 30 days.",
        ));
    }
    GuildContext::load(ports, guild_id, actor.user_id)
        .await?
        .require(Permissions::MANAGE_GUILD)?;
    let today = ports.clock.now().date_naive();
    let first = today - Duration::days(i64::from(days - 1));
    let (started, rows) = ports.guilds.membership_activity(guild_id, first).await?;
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid epoch");
    let observed_messages: Vec<_> = stream::iter((0..days).map(|offset| {
        let day = first + Duration::days(i64::from(offset));
        let ordinal = (day - epoch).num_days();
        ports.messages.guild_message_count(guild_id, ordinal)
    }))
    .buffered(4)
    .collect()
    .await;
    let mut result = Vec::with_capacity(days as usize);
    for (offset, message_result) in (0..days).zip(observed_messages) {
        let day = first + Duration::days(i64::from(offset));
        let row = rows.iter().find(|row| row.day == day);
        let observed = day >= started.date_naive();
        let membership_complete = day > started.date_naive() && day < today;
        let initialized = message_result
            .as_ref()
            .ok()
            .and_then(|(_, _, initialized)| *initialized);
        let source_day = initialized
            .and_then(|ms| i64::try_from(ms).ok())
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|at| at.date_naive());
        let message_observed = source_day.is_some_and(|source_day| day >= source_day);
        let message_complete = message_observed
            && source_day.is_some_and(|source_day| day > source_day)
            && message_result
                .as_ref()
                .is_ok_and(|(_, complete, _)| *complete)
            && day < today;
        let voice_minutes = row
            .filter(|row| row.voice_millis > 0)
            .map(|row| row.voice_millis as f64 / 60_000.0);
        result.push(Day {
            day,
            joins: observed.then(|| row.map_or(0, |row| row.joins)),
            leaves: observed.then(|| row.map_or(0, |row| row.leaves)),
            membership_complete,
            membership_coverage: if !observed {
                "unavailable"
            } else if membership_complete {
                "complete"
            } else {
                "partial"
            },
            messages: message_result
                .as_ref()
                .ok()
                .and_then(|(count, _, _)| message_observed.then_some(*count)),
            message_source_initialized_at_ms: initialized,
            activity_source_initialized_at_ms: started.timestamp_millis(),
            message_complete,
            message_coverage: if !message_observed {
                "unavailable"
            } else if message_complete {
                "complete"
            } else {
                "partial"
            },
            voice_coverage: if voice_minutes.is_some() {
                "partial"
            } else {
                "unavailable"
            },
            voice_minutes,
        });
    }
    Ok(result)
}
