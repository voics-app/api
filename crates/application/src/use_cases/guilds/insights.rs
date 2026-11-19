use crate::{
    context::{Actor, Ports},
    error::{AppError, AppResult},
    support::guild_context::GuildContext,
};
use chrono::{Duration, NaiveDate};
use domain::{permissions::Permissions, Snowflake};

#[derive(Debug)]
pub struct Day {
    pub day: NaiveDate,
    pub joins: Option<i64>,
    pub leaves: Option<i64>,
    pub membership_complete: bool,
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
    let mut result = Vec::with_capacity(days as usize);
    for offset in 0..days {
        let day = first + Duration::days(i64::from(offset));
        let row = rows.iter().find(|row| row.day == day);
        let observed = day >= started.date_naive();
        result.push(Day {
            day,
            joins: observed.then(|| row.map_or(0, |row| row.joins)),
            leaves: observed.then(|| row.map_or(0, |row| row.leaves)),
            membership_complete: day > started.date_naive() && day < today,
        });
    }
    Ok(result)
}
