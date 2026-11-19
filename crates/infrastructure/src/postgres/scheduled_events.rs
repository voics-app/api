//! Read mappings for guild scheduled events.
use super::convert::{sf, sf_opt};
use super::error::db;
use crate::events::route::Routed;
use application::{AppError, AppResult};
use chrono::{DateTime, Utc};
use contracts::envelope::{Envelope, TOPIC_USER};
use contracts::wire::{timestamp, Id};
use domain::scheduled_event::{EventKind, EventStatus, ScheduledEvent, ScheduledEventDetails};
use domain::Snowflake;
use serde_json::json;
use sqlx::{FromRow, PgConnection, PgPool};
use uuid::Uuid;

const COLUMNS: &str = "e.id, e.guild_id, e.creator_id, e.channel_id, e.name, e.description, e.scheduled_start_time, e.scheduled_end_time, e.status, e.revision, e.entity_type, e.location, e.image, (SELECT count(*) FROM guild_scheduled_event_users u WHERE u.event_id = e.id) AS user_count";

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
    revision: i64,
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
            revision: row.revision,
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

pub(super) async fn update(
    conn: &mut PgConnection,
    event: &ScheduledEvent,
    expected_revision: i64,
) -> AppResult<()> {
    let (kind, channel, location) = match &event.details.kind {
        EventKind::Stage { channel_id } => (1_i16, Some(channel_id.as_i64()), None),
        EventKind::Voice { channel_id } => (2_i16, Some(channel_id.as_i64()), None),
        EventKind::External { location } => (3_i16, None, Some(location.as_str())),
    };
    let changed = sqlx::query("UPDATE guild_scheduled_events SET channel_id=$1, name=$2, description=$3, schedule_revision=schedule_revision + CASE WHEN scheduled_start_time IS DISTINCT FROM $4 THEN 1 ELSE 0 END, scheduled_start_time=$4, scheduled_end_time=$5, status=$6, entity_type=$7, location=$8, image=$9, revision=revision+1 WHERE id=$10 AND guild_id=$11 AND revision=$12")
        .bind(channel).bind(&event.details.name).bind(&event.details.description).bind(event.details.start)
        .bind(event.details.end).bind(event.details.status.code()).bind(kind).bind(location).bind(&event.image)
        .bind(event.id.as_i64()).bind(event.guild_id.as_i64()).bind(expected_revision).execute(conn).await.map_err(db)?.rows_affected();
    if changed == 0 {
        return Err(AppError::Conflict {
            code: 0,
            message: "Event changed; reload and retry.".into(),
        });
    }
    Ok(())
}

pub(super) async fn delete(
    conn: &mut PgConnection,
    guild: Snowflake,
    event: Snowflake,
    expected_revision: i64,
) -> AppResult<()> {
    let changed = sqlx::query(
        "DELETE FROM guild_scheduled_events WHERE guild_id=$1 AND id=$2 AND revision=$3",
    )
    .bind(guild.as_i64())
    .bind(event.as_i64())
    .bind(expected_revision)
    .execute(conn)
    .await
    .map_err(db)?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::Conflict {
            code: 0,
            message: "Event changed; reload and retry.".into(),
        });
    }
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
        "INSERT INTO guild_scheduled_event_users (guild_id, event_id, user_id) SELECT $1, id, $3 FROM guild_scheduled_events WHERE guild_id = $1 AND id = $2 AND status = 1 AND scheduled_start_time > now() ON CONFLICT DO NOTHING"
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

#[derive(FromRow)]
struct DueEvent {
    id: i64,
    guild_id: i64,
    scheduled_start_time: DateTime<Utc>,
    schedule_revision: i64,
}

/// Atomically enqueues a bounded batch. Multiple replicas skip rows held by a peer.
pub async fn enqueue_due_reminders(pool: &PgPool) -> AppResult<usize> {
    const EVENTS_PER_BATCH: i64 = 20;
    const USERS_PER_EVENT: i64 = 200;
    let mut tx = pool.begin().await.map_err(db)?;
    let now = Utc::now();
    let events: Vec<DueEvent> = sqlx::query_as("SELECT id, guild_id, scheduled_start_time, schedule_revision FROM guild_scheduled_events WHERE status IN (1, 2) AND scheduled_start_time <= $1 AND reminders_processed_revision IS DISTINCT FROM schedule_revision ORDER BY scheduled_start_time, id LIMIT $2 FOR UPDATE SKIP LOCKED")
        .bind(now).bind(EVENTS_PER_BATCH).fetch_all(&mut *tx).await.map_err(db)?;
    let mut enqueued = 0;
    for event in events {
        let users: Vec<i64> = sqlx::query_scalar("SELECT u.user_id FROM guild_scheduled_event_users u WHERE u.event_id=$1 AND NOT EXISTS (SELECT 1 FROM guild_scheduled_event_reminders r WHERE r.event_id=u.event_id AND r.user_id=u.user_id AND r.event_revision=$2 AND r.scheduled_start_time=$3) ORDER BY u.user_id LIMIT $4")
            .bind(event.id).bind(event.schedule_revision).bind(event.scheduled_start_time).bind(USERS_PER_EVENT).fetch_all(&mut *tx).await.map_err(db)?;
        for user_id in &users {
            sqlx::query("INSERT INTO guild_scheduled_event_reminders (event_id, user_id, event_revision, scheduled_start_time, enqueued_at) VALUES ($1,$2,$3,$4,$5)")
                .bind(event.id).bind(user_id).bind(event.schedule_revision).bind(event.scheduled_start_time).bind(now).execute(&mut *tx).await.map_err(db)?;
            let key = format!(
                "{}:{}:{}:{}:{}",
                event.guild_id,
                event.id,
                event.scheduled_start_time.timestamp_millis(),
                event.schedule_revision,
                user_id
            );
            let data = json!({ "guild_id": event.guild_id.to_string(), "guild_scheduled_event_id": event.id.to_string(), "scheduled_start_time": timestamp(event.scheduled_start_time), "revision": event.schedule_revision, "dedupe_key": key });
            let routed = Routed {
                topic: TOPIC_USER,
                key: user_id.to_string(),
                envelope: Envelope::new(
                    Uuid::now_v7(),
                    "GUILD_SCHEDULED_EVENT_START",
                    now,
                    Some(Id(sf(event.guild_id))),
                    None,
                    Some(vec![Id(sf(*user_id))]),
                    data,
                ),
            };
            super::outbox::enqueue(&mut tx, &routed, now).await?;
            enqueued += 1;
        }
        if users.len() < USERS_PER_EVENT as usize {
            sqlx::query(
                "UPDATE guild_scheduled_events SET reminders_processed_revision=$2 WHERE id=$1",
            )
            .bind(event.id)
            .bind(event.schedule_revision)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        }
    }
    tx.commit().await.map_err(db)?;
    Ok(enqueued)
}
