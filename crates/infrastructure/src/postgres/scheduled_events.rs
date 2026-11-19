//! Read mappings for guild scheduled events.
use super::convert::{sf, sf_opt};
use super::error::db;
use application::{AppError, AppResult};
use chrono::{DateTime, Utc};
use domain::scheduled_event::{EventKind, EventStatus, ScheduledEvent, ScheduledEventDetails};
use domain::Snowflake;
use sqlx::{FromRow, PgConnection, PgPool};

const COLUMNS: &str = "e.id, e.guild_id, e.creator_id, e.channel_id, e.name, e.description, e.scheduled_start_time, e.scheduled_end_time, e.status, e.entity_type, e.location, e.image, (SELECT count(*) FROM guild_scheduled_event_users u WHERE u.event_id = e.id) AS user_count";

#[derive(FromRow)]
struct Row {
    id: i64,
    guild_id: i64,
    creator_id: Option<i64>,
    channel_id: Option<i64>,
    name: String,
    description: Option<String>,
    scheduled_start_time: DateTime<Utc>,
    scheduled_end_time: Option<DateTime<Utc>>,
    status: i16,
    entity_type: i16,
    location: Option<String>,
    image: Option<String>,
    user_count: i64,
}

impl TryFrom<Row> for ScheduledEvent {
    type Error = AppError;
    fn try_from(row: Row) -> AppResult<Self> {
        let kind = match (row.entity_type, row.channel_id, row.location) {
            (1, Some(id), None) => EventKind::Stage { channel_id: sf(id) },
            (2, Some(id), None) => EventKind::Voice { channel_id: sf(id) },
            (3, None, Some(location)) => EventKind::External { location },
            _ => return Err(AppError::internal("invalid stored event kind")),
        };
        Ok(Self {
            id: sf(row.id),
            guild_id: sf(row.guild_id),
            creator_id: sf_opt(row.creator_id),
            details: ScheduledEventDetails {
                name: row.name,
                description: row.description,
                start: row.scheduled_start_time,
                end: row.scheduled_end_time,
                kind,
                status: EventStatus::from_code(row.status)
                    .ok_or_else(|| AppError::internal("invalid stored event status"))?,
            },
            image: row.image,
            user_count: u64::try_from(row.user_count).map_err(AppError::internal)?,
        })
    }
}

pub(super) async fn list(pool: &PgPool, guild_id: Snowflake) -> AppResult<Vec<ScheduledEvent>> {
    let rows: Vec<Row> = sqlx::query_as(&format!("SELECT {COLUMNS} FROM guild_scheduled_events e WHERE e.guild_id = $1 ORDER BY e.scheduled_start_time, e.id"))
        .bind(guild_id.as_i64()).fetch_all(pool).await.map_err(db)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

pub(super) async fn find(
    pool: &PgPool,
    guild_id: Snowflake,
    event_id: Snowflake,
) -> AppResult<Option<ScheduledEvent>> {
    let row: Option<Row> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM guild_scheduled_events e WHERE e.guild_id = $1 AND e.id = $2"
    ))
    .bind(guild_id.as_i64())
    .bind(event_id.as_i64())
    .fetch_optional(pool)
    .await
    .map_err(db)?;
    row.map(TryInto::try_into).transpose()
}

pub(super) async fn interested(
    pool: &PgPool,
    event_id: Snowflake,
    user_id: Snowflake,
) -> AppResult<bool> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM guild_scheduled_event_users WHERE event_id = $1 AND user_id = $2)")
        .bind(event_id.as_i64()).bind(user_id.as_i64()).fetch_one(pool).await.map_err(db)
}

pub(super) async fn insert(conn: &mut PgConnection, event: &ScheduledEvent) -> AppResult<()> {
    let (kind, channel, location) = match &event.details.kind {
        EventKind::Stage { channel_id } => (1_i16, Some(channel_id.as_i64()), None),
        EventKind::Voice { channel_id } => (2_i16, Some(channel_id.as_i64()), None),
        EventKind::External { location } => (3_i16, None, Some(location.as_str())),
    };
    sqlx::query("INSERT INTO guild_scheduled_events (id, guild_id, creator_id, channel_id, name, description, scheduled_start_time, scheduled_end_time, status, entity_type, location, image) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
        .bind(event.id.as_i64()).bind(event.guild_id.as_i64()).bind(event.creator_id.map(|id| id.as_i64()))
        .bind(channel).bind(&event.details.name).bind(&event.details.description)
        .bind(event.details.start).bind(event.details.end).bind(event.details.status.code())
        .bind(kind).bind(location).bind(&event.image).execute(conn).await.map_err(db)?;
    Ok(())
}

pub(super) async fn set_interest(
    conn: &mut PgConnection,
    guild: Snowflake,
    event: Snowflake,
    user: Snowflake,
    interested: bool,
) -> AppResult<bool> {
    let query = if interested {
        "INSERT INTO guild_scheduled_event_users (guild_id, event_id, user_id) SELECT $1, id, $3 FROM guild_scheduled_events WHERE guild_id = $1 AND id = $2 AND status = 1 ON CONFLICT DO NOTHING"
    } else {
        "DELETE FROM guild_scheduled_event_users WHERE guild_id = $1 AND event_id = $2 AND user_id = $3"
    };
    Ok(sqlx::query(query)
        .bind(guild.as_i64())
        .bind(event.as_i64())
        .bind(user.as_i64())
        .execute(conn)
        .await
        .map_err(db)?
        .rows_affected()
        != 0)
}
