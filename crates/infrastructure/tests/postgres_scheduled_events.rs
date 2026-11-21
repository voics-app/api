//! Persistence checks for scheduled-event reads and membership cleanup.
mod support;

use application::ports::GuildRepository;
use application::ports::UnitOfWork;
use application::write_set::{Mutation, WriteSet};
use domain::scheduled_event::EventKind;
use domain::Snowflake;
use infrastructure::postgres::PgStore;
use infrastructure::postgres::PgUnitOfWork;

#[tokio::test]
async fn interest_counts_and_membership_cleanup_round_trip() {
    let database = support::start_database().await;
    let pool = &database.pool;
    sqlx::query("INSERT INTO users (id, username, created_at) VALUES (1, 'owner', now()), (2, 'guest', now())")
        .execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO guilds (id, name, owner_id, created_at) VALUES (10, 'guild', 1, now())",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO members (guild_id, user_id, joined_at) VALUES (10, 1, now()), (10, 2, now())",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO guild_scheduled_events (id, guild_id, creator_id, name, scheduled_start_time, scheduled_end_time, status, entity_type, location) VALUES (20, 10, 1, 'Meetup', now() + interval '1 hour', now() + interval '2 hours', 1, 3, 'Park')")
        .execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO guild_scheduled_event_users (guild_id, event_id, user_id) VALUES (10, 20, 2)",
    )
    .execute(pool)
    .await
    .unwrap();

    let store = PgStore::new(pool.clone());
    let guild = Snowflake::new(10);
    let event = Snowflake::new(20);
    let guest = Snowflake::new(2);
    let loaded = store.scheduled_event(guild, event).await.unwrap().unwrap();
    assert_eq!(loaded.user_count, 1);
    assert_eq!(
        loaded.details.kind,
        EventKind::External {
            location: "Park".into()
        }
    );
    assert!(store
        .scheduled_event_interested(event, guest)
        .await
        .unwrap());
    assert_eq!(store.scheduled_events(guild).await.unwrap().len(), 1);

    sqlx::query("INSERT INTO guild_scheduled_event_reminders (event_id, user_id, event_revision, scheduled_start_time, enqueued_at) SELECT id, 2, schedule_revision, scheduled_start_time, now() FROM guild_scheduled_events WHERE id = 20")
        .execute(pool).await.unwrap();

    sqlx::query("DELETE FROM members WHERE guild_id = 10 AND user_id = 2")
        .execute(pool)
        .await
        .unwrap();
    assert!(!store
        .scheduled_event_interested(event, guest)
        .await
        .unwrap());
    assert_eq!(
        store
            .scheduled_event(guild, event)
            .await
            .unwrap()
            .unwrap()
            .user_count,
        0
    );
    let reminder_exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM guild_scheduled_event_reminders WHERE event_id = 20 AND user_id = 2)")
        .fetch_one(pool).await.unwrap();
    assert!(
        reminder_exists,
        "leaving must not erase notification deduplication"
    );
}

#[tokio::test]
async fn external_event_cannot_be_stored_without_a_location() {
    let database = support::start_database().await;
    let pool = &database.pool;
    sqlx::query("INSERT INTO users (id, username, created_at) VALUES (1, 'owner', now())")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO guilds (id, name, owner_id, created_at) VALUES (10, 'guild', 1, now())",
    )
    .execute(pool)
    .await
    .unwrap();
    let result = sqlx::query("INSERT INTO guild_scheduled_events (id, guild_id, name, scheduled_start_time, scheduled_end_time, status, entity_type) VALUES (20, 10, 'Meetup', now() + interval '1 hour', now() + interval '2 hours', 1, 3)")
        .execute(pool).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn competing_workers_enqueue_one_reminder_per_schedule_and_user() {
    let database = support::start_database().await;
    let pool = &database.pool;
    sqlx::query("INSERT INTO users (id, username, created_at) VALUES (1, 'owner', now())")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO guilds (id, name, owner_id, created_at) VALUES (10, 'guild', 1, now())",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO members (guild_id, user_id, joined_at) VALUES (10, 1, now())")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO sessions (id, user_id, created_at) VALUES ('00000000-0000-0000-0000-000000000001', 1, now())").execute(pool).await.unwrap();
    sqlx::query("INSERT INTO push_tokens (session_id, user_id, platform, token, environment, created_at, updated_at) VALUES ('00000000-0000-0000-0000-000000000001', 1, 'fcm', 'fixture-token', 'production', now(), now())").execute(pool).await.unwrap();
    sqlx::query("INSERT INTO guild_scheduled_events (id, guild_id, name, scheduled_start_time, scheduled_end_time, status, entity_type, location) VALUES (20, 10, 'Meetup', now() - interval '1 minute', now() + interval '1 hour', 1, 3, 'Park')")
        .execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO guild_scheduled_event_users (guild_id, event_id, user_id) VALUES (10, 20, 1)",
    )
    .execute(pool)
    .await
    .unwrap();
    let (first, second) = tokio::join!(
        infrastructure::postgres::enqueue_due_reminders(pool),
        infrastructure::postgres::enqueue_due_reminders(pool)
    );
    assert_eq!(first.unwrap() + second.unwrap(), 1);
    assert_eq!(
        infrastructure::postgres::enqueue_due_reminders(pool)
            .await
            .unwrap(),
        0
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE payload->>'type' = 'GUILD_SCHEDULED_EVENT_START' AND payload->'data'->>'guild_scheduled_event_id' = '20'")
        .fetch_one(pool).await.unwrap();
    assert_eq!(count, 1);
    let store = PgStore::new(pool.clone());
    let claim = store
        .claim_scheduled_event_delivery()
        .await
        .unwrap()
        .unwrap();
    assert!(store
        .claim_scheduled_event_delivery()
        .await
        .unwrap()
        .is_none());
    assert!(store
        .finish_scheduled_event_delivery(claim.id, claim.lease_id, Some(1))
        .await
        .unwrap());
    sqlx::query("UPDATE guild_scheduled_event_push_deliveries SET next_attempt_at=now()-interval '1 second' WHERE id=$1").bind(claim.id).execute(pool).await.unwrap();
    let retry = store
        .claim_scheduled_event_delivery()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retry.id, claim.id);
    assert_ne!(retry.lease_id, claim.lease_id);
    assert!(!store
        .finish_scheduled_event_delivery(claim.id, claim.lease_id, None)
        .await
        .unwrap());
    assert!(store
        .finish_scheduled_event_delivery(retry.id, retry.lease_id, None)
        .await
        .unwrap());
    assert!(store
        .claim_scheduled_event_delivery()
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn interest_waiting_on_event_lock_rechecks_the_start_time() {
    let database = support::start_database().await;
    let pool = &database.pool;
    sqlx::query("INSERT INTO users (id, username, created_at) VALUES (1, 'owner', now())")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO guilds (id, name, owner_id, created_at) VALUES (10, 'guild', 1, now())",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO members (guild_id, user_id, joined_at) VALUES (10, 1, now())")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO guild_scheduled_events (id, guild_id, name, scheduled_start_time, scheduled_end_time, status, entity_type, location) VALUES (20, 10, 'Meetup', now() + interval '1 hour', now() + interval '2 hours', 1, 3, 'Park')").execute(pool).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM guild_scheduled_events WHERE id = 20 FOR UPDATE")
        .execute(&mut *tx)
        .await
        .unwrap();
    let uow = PgUnitOfWork::new(pool.clone(), "nl-home".into());
    let waiting = tokio::spawn(async move {
        let mut writes = WriteSet::new();
        writes.mutate(Mutation::SetScheduledEventInterest {
            guild_id: Snowflake::new(10),
            event_id: Snowflake::new(20),
            user_id: Snowflake::new(1),
            channel_id: None,
            interested: true,
        });
        uow.commit(writes).await
    });
    tokio::task::yield_now().await;
    sqlx::query("UPDATE guild_scheduled_events SET scheduled_start_time = now() - interval '1 minute' WHERE id = 20").execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    assert!(matches!(
        waiting.await.unwrap(),
        Err(application::AppError::Conflict { .. })
    ));
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM guild_scheduled_event_users WHERE event_id=20")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
}
