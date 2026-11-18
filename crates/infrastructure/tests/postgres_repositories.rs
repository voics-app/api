//! Integration tests against a real Postgres started with testcontainers.

use std::time::Duration;

use chrono::Utc;
use sqlx::PgPool;
use testcontainers::core::{ContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};

use application::events::DomainEvent;
use application::ports::{
    AuditLogQuery, AuditLogRepository, ChannelRepository, GuildRepository, InviteRepository,
    UnitOfWork, UserRepository,
};
use application::write_set::{Mutation, WriteSet};
use domain::audit::{AuditAction, AuditLogEntry};
use domain::channel::{Channel, ChannelType};
use domain::guild::{Guild, Member, Role};
use domain::invite::Invite;
use domain::permissions::{OverwriteKind, PermissionOverwrite, Permissions};
use domain::user::{User, UserSettings};
use domain::Snowflake;
use infrastructure::postgres::{connect, migrate, PgStore, PgUnitOfWork};

fn user(id: u64, name: &str) -> User {
    User {
        id: Snowflake::new(id),
        username: name.to_owned(),
        global_name: None,
        avatar: None,
        bio: None,
        banner: None,
        pronouns: None,
        theme_colors: None,
        accent_color: None,
        bot: false,
        system: false,
        email: Some(format!("{name}@example.com")),
        verified: true,
        mfa_enabled: false,
        flags: 0,
        platform_admin: false,
        suspension: None,
        created_at: Utc::now(),
    }
}

async fn database_pool(port: u16) -> PgPool {
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/discordy");
    for _ in 0..60 {
        if let Ok(pool) = connect(&url, 4).await {
            if sqlx::query("SELECT 1").execute(&pool).await.is_ok() {
                return pool;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("postgres did not become ready");
}

#[tokio::test]
async fn guild_lifecycle_round_trips_through_postgres() {
    let container = GenericImage::new("postgres", "16-alpine")
        .with_exposed_port(ContainerPort::Tcp(5432))
        .with_wait_for(WaitFor::message_on_stderr(
            "database system is ready to accept connections",
        ))
        .with_env_var("POSTGRES_PASSWORD", "postgres")
        .with_env_var("POSTGRES_DB", "discordy")
        .with_startup_timeout(Duration::from_secs(300))
        .start()
        .await
        .expect("start postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("mapped port");
    let pool = database_pool(port).await;
    migrate(&pool).await.expect("migrations apply");

    let store = PgStore::new(pool.clone());
    let uow = PgUnitOfWork::new(pool.clone(), "nl-home".into());
    let now = Utc::now();
    let alice = Snowflake::new(10);
    let bob = Snowflake::new(11);
    let guild_id = Snowflake::new(100);

    let mut users = WriteSet::new();
    users.mutate(Mutation::InsertUser {
        user: user(10, "alice"),
        credentials: None,
        settings: Box::new(UserSettings::defaults_for(alice)),
    });
    users.mutate(Mutation::InsertUser {
        user: user(11, "bob"),
        credentials: None,
        settings: Box::new(UserSettings::defaults_for(bob)),
    });
    uow.commit(users).await.expect("users committed");

    let found = UserRepository::find(&store, alice)
        .await
        .expect("query")
        .expect("alice exists");
    assert_eq!(found.username, "alice");
    assert_eq!(
        store
            .find_by_email("BOB@example.com")
            .await
            .expect("query")
            .map(|u| u.id),
        Some(bob)
    );

    let guild = Guild {
        id: guild_id,
        name: "Voics".into(),
        icon: None,
        description: None,
        owner_id: alice,
        voice_region: None,
        verification_level: 0,
        default_message_notifications: 0,
        explicit_content_filter: 0,
        afk_channel_id: None,
        afk_timeout: 300,
        system_channel_id: None,
        system_channel_flags: 0,
        banner: None,
        splash: None,
        prune_admin_only: false,
        audit_policy: Default::default(),
        created_at: now,
    };
    let everyone = Role {
        id: guild_id,
        guild_id,
        name: "@everyone".into(),
        color: 0,
        hoist: false,
        icon: None,
        unicode_emoji: None,
        position: 0,
        permissions: Permissions::DEFAULT_EVERYONE,
        managed: false,
        mentionable: false,
        bot_id: None,
    };
    let mut channel = Channel::new(
        Snowflake::new(101),
        ChannelType::GuildText,
        Some(guild_id),
        now,
    );
    channel.name = Some("general".into());
    channel.overwrites = vec![PermissionOverwrite {
        id: guild_id,
        kind: OverwriteKind::Role,
        allow: Permissions::from_bits(0),
        deny: Permissions::VIEW_CHANNEL,
    }];
    let invite = Invite {
        code: "abc12345".into(),
        guild_id,
        channel_id: channel.id,
        inviter_id: Some(alice),
        max_age: 0,
        max_uses: 1,
        uses: 0,
        temporary: false,
        created_at: now,
    };

    let mut writes = WriteSet::new();
    writes.mutate(Mutation::InsertGuild(guild.clone()));
    writes.mutate(Mutation::InsertRole(everyone));
    writes.mutate(Mutation::UpsertMember(Member::new(guild_id, alice, now)));
    writes.mutate(Mutation::UpsertMember(Member::new(guild_id, bob, now)));
    writes.mutate(Mutation::InsertChannel(channel.clone()));
    writes.mutate(Mutation::InsertInvite(invite));
    writes.emit(DomainEvent::ChannelCreate(channel.clone()));
    writes.audit(AuditLogEntry {
        id: Snowflake::new(200),
        guild_id,
        user_id: Some(alice),
        target_id: Some(channel.id),
        action: AuditAction::ChannelCreate,
        changes: vec![],
        reason: None,
        created_at: now,
    });
    uow.commit(writes).await.expect("guild committed");

    assert_eq!(
        GuildRepository::find(&store, guild_id)
            .await
            .expect("query")
            .map(|g| g.name),
        Some("Voics".into())
    );
    assert_eq!(store.member_count(guild_id).await.expect("count"), 2);
    let members = store
        .member_profiles(guild_id, 10, None)
        .await
        .expect("profiles");
    assert_eq!(members.len(), 2);
    assert_eq!(members[0].user.username, "alice");

    let channels = ChannelRepository::list_for_guild(&store, guild_id)
        .await
        .expect("channels");
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].overwrites, channel.overwrites);

    let audit = store
        .list(
            guild_id,
            AuditLogQuery {
                limit: 10,
                ..AuditLogQuery::default()
            },
        )
        .await
        .expect("audit");
    assert_eq!(audit.len(), 1);

    let (outbox_rows,): (i64,) = sqlx::query_as("SELECT count(*) FROM outbox")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(
        outbox_rows, 2,
        "channel create and the audit entry each enqueue one envelope"
    );

    let mut consume = WriteSet::new();
    consume.mutate(Mutation::ConsumeInvite("abc12345".into()));
    uow.commit(consume.clone()).await.expect("first use");
    assert!(
        uow.commit(consume).await.is_err(),
        "exhausted invite must be rejected"
    );
    assert_eq!(
        InviteRepository::find(&store, "abc12345")
            .await
            .expect("query")
            .map(|i| i.uses),
        Some(1)
    );
}
