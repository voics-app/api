use super::*;
use http_body_util::BodyExt;
use tower::ServiceExt;

const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

#[tokio::test]
async fn webhook_credentials_are_private_uncacheable_audited_and_rollback_with_the_outbox() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = router(ports(&database.pool, &public, &private));
    let owner = register(&app, "manager").await;
    let outsider = register(&app, "outsider").await;
    let (guild, channel) = guild(&app, &owner).await;
    let response = app
        .clone()
        .oneshot(request(
            Method::POST,
            &format!("/api/v1/channels/{channel}/webhooks"),
            Some(&owner),
            Some(json!({"name":"Reminders","avatar":PNG})),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let hook: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = hook["id"].as_str().unwrap();
    let old = hook["token"].as_str().unwrap();
    assert!(hook["avatar"].is_string());
    assert_eq!(
        call(
            &app,
            Method::POST,
            &format!("/api/v1/webhooks/{id}/reset-token"),
            &outsider,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            &app,
            request(
                Method::POST,
                &format!("/api/v1/webhooks/{id}/reset-token"),
                None,
                None
            )
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let listed = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild}/webhooks"),
        &owner,
        None,
    )
    .await
    .1;
    assert!(listed[0].get("token").is_none() && listed[0].get("url").is_none());
    let edited = call(
        &app,
        Method::PATCH,
        &format!("/api/v1/webhooks/{id}"),
        &owner,
        Some(json!({"avatar":null})),
    )
    .await;
    assert_eq!(edited.0, StatusCode::OK);
    assert!(edited.1["avatar"].is_null());
    let (_, other_channel) = super::guild(&app, &owner).await;
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &format!("/api/v1/webhooks/{id}"),
            &owner,
            Some(json!({"channel_id":other_channel}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    let response = app
        .clone()
        .oneshot(request(
            Method::POST,
            &format!("/api/v1/webhooks/{id}/reset-token"),
            Some(&owner),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let rotated: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let new = rotated["token"].as_str().unwrap();
    let audit = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild}/audit-logs?target_id={id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(audit.0, StatusCode::OK);
    let serialized = audit.1.to_string();
    assert!(serialized.contains("token_rotated"));
    assert!(
        !serialized.contains(old)
            && !serialized.contains(new)
            && !serialized.contains("token_hash")
    );

    sqlx::raw_sql("CREATE FUNCTION reject_test_outbox() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'test outbox failure'; END $$; CREATE TRIGGER reject_test_outbox BEFORE INSERT ON outbox FOR EACH ROW EXECUTE FUNCTION reject_test_outbox();").execute(&database.pool).await.unwrap();
    assert_eq!(
        call(
            &app,
            Method::POST,
            &format!("/api/v1/webhooks/{id}/reset-token"),
            &owner,
            None
        )
        .await
        .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        execute_hook(&app, id, new).await,
        StatusCode::OK,
        "rollback retains the last committed credential"
    );
    sqlx::query("DROP TRIGGER reject_test_outbox ON outbox")
        .execute(&database.pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/v1/webhooks/{id}"),
            &owner,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            Method::POST,
            &format!("/api/v1/webhooks/{id}/reset-token"),
            &owner,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn concurrent_webhook_metadata_edits_preserve_omitted_fields_and_return_committed_metadata() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let mut ports = ports(&database.pool, &public, &private);
    let paused = Arc::new(PauseCommit {
        inner: ports.unit_of_work.clone(),
        matches: |m| matches!(m, Mutation::UpdateWebhook { .. }),
        armed: AtomicBool::new(false),
        reached: Notify::new(),
        release: Notify::new(),
    });
    ports.unit_of_work = paused.clone();
    let app = router(ports);
    let owner = register(&app, "manager").await;
    let (_, channel) = guild(&app, &owner).await;
    let (id, _) = webhook(&app, &owner, &channel).await;
    paused.armed.store(true, Ordering::SeqCst);
    let editing = {
        let app = app.clone();
        let owner = owner.clone();
        let id = id.clone();
        tokio::spawn(async move {
            call(
                &app,
                Method::PATCH,
                &format!("/api/v1/webhooks/{id}"),
                &owner,
                Some(json!({"name":"Changed name"})),
            )
            .await
        })
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        paused.reached.notified(),
    )
    .await
    .unwrap();
    let changed_avatar = call(
        &app,
        Method::PATCH,
        &format!("/api/v1/webhooks/{id}"),
        &owner,
        Some(json!({"avatar":PNG})),
    )
    .await;
    paused.release.notify_one();
    assert_eq!(changed_avatar.0, StatusCode::OK);
    let edited = editing.await.unwrap();
    assert_eq!(edited.0, StatusCode::OK);
    assert_eq!(edited.1["name"], "Changed name");
    assert!(edited.1["avatar"].is_string());
    assert_eq!(edited.1["avatar"], changed_avatar.1["avatar"]);
}
