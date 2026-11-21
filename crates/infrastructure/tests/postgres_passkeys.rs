//! Storage constraints remain correct when independent API replicas race.
mod support;
use application::ports::{PasskeyRecord, VerifiedCredential};
use domain::account::{AccountToken, TokenPurpose};
use serde_json::json;
use support::passkeys::fixture;
use uuid::Uuid;

fn key(user_id: domain::Snowflake) -> PasskeyRecord {
    let id = Uuid::now_v7();
    PasskeyRecord {
        id,
        user_id,
        name: "Key".into(),
        credential: VerifiedCredential {
            id: id.as_bytes().to_vec(),
            material: json!({"test":"storage-only"}),
            counter: 0,
        },
        created_at: chrono::Utc::now(),
        last_used_at: None,
    }
}

async fn ticket(f: &support::passkeys::Fixture, hash: &str) {
    f.ports
        .account_tokens
        .create(&AccountToken {
            token_hash: hash.into(),
            user_id: f.user_id,
            purpose: TokenPurpose::MfaLogin,
            email: None,
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn concurrent_registration_enforces_limit_and_returns_one_initial_recovery_set() {
    let f = fixture().await;
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let repository = f.ports.passkeys.clone();
        let key = key(f.user_id);
        tasks.spawn(async move {
            repository
                .register(&key, &["storage-test-hash".into()])
                .await
        });
    }
    let mut inserted = 0;
    let mut recovery_sets = 0;
    while let Some(result) = tasks.join_next().await {
        if let Ok(recovery) = result.unwrap() {
            inserted += 1;
            recovery_sets += usize::from(recovery);
        }
    }
    assert_eq!(inserted, 10);
    assert_eq!(recovery_sets, 1);
    assert_eq!(f.ports.passkeys.list(f.user_id).await.unwrap().len(), 10);
    assert!(
        f.ports
            .users
            .find(f.user_id)
            .await
            .unwrap()
            .unwrap()
            .mfa_enabled
    );
}

#[tokio::test]
async fn counter_and_ticket_commit_together_and_zero_counter_authenticators_work() {
    let f = fixture().await;
    let key = key(f.user_id);
    f.ports
        .passkeys
        .register(&key, &["storage-test-hash".into()])
        .await
        .unwrap();
    ticket(&f, "zero-a").await;
    ticket(&f, "zero-b").await;
    for hash in ["zero-a", "zero-b"] {
        assert!(f
            .ports
            .passkeys
            .authenticate(f.user_id, &key.credential, hash, chrono::Utc::now())
            .await
            .unwrap());
    }
    ticket(&f, "counter-a").await;
    ticket(&f, "counter-b").await;
    let mut next = key.credential.clone();
    next.counter = 1;
    let now = chrono::Utc::now();
    let (a, b) = tokio::join!(
        f.ports
            .passkeys
            .authenticate(f.user_id, &next, "counter-a", now),
        f.ports
            .passkeys
            .authenticate(f.user_id, &next, "counter-b", now)
    );
    assert_ne!(
        a.unwrap(),
        b.unwrap(),
        "the same nonzero counter cannot commit twice"
    );
    let active: i64 = sqlx::query_scalar("SELECT count(*) FROM account_tokens WHERE token_hash IN ('counter-a','counter-b') AND used_at IS NULL")
        .fetch_one(&f.database.pool).await.unwrap();
    assert_eq!(active, 1, "the losing ticket consumption rolled back");
    let hash = if f
        .ports
        .account_tokens
        .find_active("counter-a", TokenPurpose::MfaLogin, now)
        .await
        .unwrap()
        .is_some()
    {
        "counter-a"
    } else {
        "counter-b"
    };
    assert!(
        !f.ports
            .passkeys
            .authenticate(f.user_id, &key.credential, hash, now)
            .await
            .unwrap(),
        "a counter cannot reset to zero"
    );
    next.counter = 2;
    assert!(f
        .ports
        .passkeys
        .authenticate(f.user_id, &next, hash, now)
        .await
        .unwrap());
}

#[tokio::test]
async fn failed_outbox_insert_rolls_back_factor_activation_and_removal() {
    let f = fixture().await;
    let key = key(f.user_id);
    sqlx::query("ALTER TABLE outbox ADD CONSTRAINT reject_factor_change CHECK (payload->>'type' <> 'USER_UPDATE')").execute(&f.database.pool).await.unwrap();
    assert!(f
        .ports
        .passkeys
        .register(&key, &["storage-test-hash".into()])
        .await
        .is_err());
    assert!(f.ports.passkeys.list(f.user_id).await.unwrap().is_empty());
    assert!(
        !f.ports
            .users
            .find(f.user_id)
            .await
            .unwrap()
            .unwrap()
            .mfa_enabled
    );
    assert!(f
        .ports
        .users
        .backup_codes(f.user_id)
        .await
        .unwrap()
        .is_empty());
    sqlx::query("ALTER TABLE outbox DROP CONSTRAINT reject_factor_change")
        .execute(&f.database.pool)
        .await
        .unwrap();
    f.ports
        .passkeys
        .register(&key, &["storage-test-hash".into()])
        .await
        .unwrap();
    sqlx::query("DELETE FROM outbox WHERE payload->>'type' = 'USER_UPDATE'")
        .execute(&f.database.pool)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE outbox ADD CONSTRAINT reject_factor_change CHECK (payload->>'type' <> 'USER_UPDATE')").execute(&f.database.pool).await.unwrap();
    assert!(f.ports.passkeys.remove(f.user_id, key.id).await.is_err());
    assert_eq!(f.ports.passkeys.list(f.user_id).await.unwrap().len(), 1);
    assert!(
        f.ports
            .users
            .find(f.user_id)
            .await
            .unwrap()
            .unwrap()
            .mfa_enabled
    );
    assert_eq!(
        f.ports.users.backup_codes(f.user_id).await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn profile_snapshot_cannot_overwrite_or_publish_stale_mfa_state() {
    use application::{
        events::DomainEvent,
        write_set::{Mutation, WriteSet},
    };
    let f = fixture().await;
    let mut stale_user = f.ports.users.find(f.user_id).await.unwrap().unwrap();
    f.ports
        .passkeys
        .register(&key(f.user_id), &["storage-test-hash".into()])
        .await
        .unwrap();
    stale_user.global_name = Some("New display name".into());
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpdateUser(stale_user.clone()))
        .emit(DomainEvent::UserUpdate(stale_user));
    f.ports.unit_of_work.commit(writes).await.unwrap();
    assert!(
        f.ports
            .users
            .find(f.user_id)
            .await
            .unwrap()
            .unwrap()
            .mfa_enabled
    );
    let payload: serde_json::Value = sqlx::query_scalar("SELECT payload FROM outbox WHERE payload->>'type' = 'USER_UPDATE' ORDER BY id DESC LIMIT 1")
        .fetch_one(&f.database.pool).await.unwrap();
    assert_eq!(
        payload["data"]["mfa_enabled"], true,
        "gateway must receive the committed factor state"
    );
}

#[tokio::test]
async fn challenge_expiry_prevents_consumption() {
    use application::ports::{ChallengeKind, PasskeyChallenge};
    let f = fixture().await;
    let now = chrono::Utc::now();
    let challenge = PasskeyChallenge {
        id: Uuid::now_v7(),
        user_id: f.user_id,
        kind: ChallengeKind::Registration,
        binding: "session-binding".into(),
        name: Some("key".into()),
        state: json!({}),
        expires_at: now + chrono::Duration::seconds(300),
    };
    f.ports
        .passkeys
        .create_challenge(&challenge, now)
        .await
        .unwrap();
    assert!(
        f.ports
            .passkeys
            .take_challenge(
                challenge.id,
                ChallengeKind::Registration,
                &challenge.binding,
                challenge.expires_at
            )
            .await
            .unwrap()
            .is_none(),
        "a ceremony is expired at its exact deadline"
    );
}

#[tokio::test]
async fn challenge_single_use_has_one_winner_under_concurrent_consumption() {
    use application::ports::{ChallengeKind, PasskeyChallenge};
    let f = fixture().await;
    let now = chrono::Utc::now();
    let challenge = PasskeyChallenge {
        id: Uuid::now_v7(),
        user_id: f.user_id,
        kind: ChallengeKind::Authentication,
        binding: "ticket-binding".into(),
        name: None,
        state: json!({}),
        expires_at: now + chrono::Duration::seconds(300),
    };
    f.ports
        .passkeys
        .create_challenge(&challenge, now)
        .await
        .unwrap();
    assert!(f
        .ports
        .passkeys
        .take_challenge(
            challenge.id,
            ChallengeKind::Authentication,
            "wrong-binding",
            now
        )
        .await
        .unwrap()
        .is_none());
    let (a, b) = tokio::join!(
        f.ports.passkeys.take_challenge(
            challenge.id,
            ChallengeKind::Authentication,
            &challenge.binding,
            now
        ),
        f.ports.passkeys.take_challenge(
            challenge.id,
            ChallengeKind::Authentication,
            &challenge.binding,
            now
        )
    );
    assert_ne!(
        a.unwrap().is_some(),
        b.unwrap().is_some(),
        "a ceremony must have exactly one consumer"
    );
}
