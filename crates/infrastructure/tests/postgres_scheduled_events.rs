//! Persistence checks for scheduled-event reads and membership cleanup.
mod support;

use application::ports::GuildRepository;
use domain::scheduled_event::EventKind;
use domain::Snowflake;
use infrastructure::postgres::PgStore;

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

    sqlx::query("INSERT INTO guild_scheduled_event_reminders (event_id, user_id, enqueued_at) VALUES (20, 2, now())")
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
