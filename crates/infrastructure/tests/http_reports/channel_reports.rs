use super::*;

fn channel_report(w: &World) -> Value {
    json!({"target": "channel", "channel_id": w.channel_id, "category": "sexual_content", "note": "Please review the channel settings."})
}

#[tokio::test]
async fn channel_reports_snapshot_only_server_metadata_without_message_history() {
    let database = start_database().await;
    let w = world(&database, None).await;
    let (status, channel) = call(
        &w.app,
        Method::PATCH,
        &format!("/api/v1/channels/{}", w.channel_id),
        &w.owner,
        Some(json!({"name": "sensitive-原文", "nsfw": true, "topic": "not report evidence"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        channel["parent_id"].is_string(),
        "the default text channel belongs to a category"
    );
    let (status, _) = call(
        &w.app,
        Method::PUT,
        &format!(
            "/api/v1/channels/{}/permissions/{}",
            w.channel_id, w.guild_id
        ),
        &w.owner,
        Some(json!({"type": 0, "allow": "1024", "deny": "65536"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        w.report(&w.ben, channel_report(&w)).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        w.report(&w.ben, channel_report(&w)).await.0,
        StatusCode::NO_CONTENT
    );
    let page = w.queue("").await;
    let reports = page["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 1);
    let report = &reports[0];
    assert_eq!(report["kind"], "channel");
    assert_eq!(report["reported"]["id"], w.owner_id);
    assert_eq!(report["content_source"], "server");
    assert_eq!(report["message_id"], Value::Null);
    assert_eq!(report["delivery_mode"], Value::Null);
    let metadata: Value = serde_json::from_str(report["content"].as_str().unwrap()).unwrap();
    assert_eq!(
        metadata,
        json!({"id": w.channel_id, "guild_id": w.guild_id, "parent_id": channel["parent_id"], "name": "sensitive-原文", "type": 0, "nsfw": true})
    );
    assert_eq!(
        w.messages
            .get_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );

    // Metadata reports do not pretend to accuse the owner of writing a message.
    assert_eq!(
        w.report(&w.owner, channel_report(&w)).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(w.queue("").await["reports"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn channel_reports_recheck_access_and_reject_submitted_content_and_other_targets() {
    let database = start_database().await;
    let w = world(&database, None).await;
    for field in ["content", "message_id", "author_id", "user_id"] {
        let mut body = channel_report(&w);
        body[field] = json!("7001");
        assert_eq!(w.report(&w.ben, body).await.0, StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        w.report(&w.ben, channel_report(&w)).await.0,
        StatusCode::NO_CONTENT
    );
    let (status, _) = call(
        &w.app,
        Method::PUT,
        &format!(
            "/api/v1/channels/{}/permissions/{}",
            w.channel_id, w.guild_id
        ),
        &w.owner,
        Some(json!({"type": 0, "allow": "0", "deny": "1024"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        w.report(&w.ben, channel_report(&w)).await.0,
        StatusCode::FORBIDDEN,
        "an existing report does not bypass current access"
    );
    let (outsider, _) = register(&w.app, "outsider").await;
    assert_eq!(
        w.report(&outsider, channel_report(&w)).await.0,
        StatusCode::FORBIDDEN
    );
    let mut missing = channel_report(&w);
    missing["channel_id"] = json!("9999999");
    assert_eq!(w.report(&w.ben, missing).await.0, StatusCode::NOT_FOUND);
    assert_eq!(w.queue("").await["reports"].as_array().unwrap().len(), 1);
    assert_eq!(
        w.messages
            .get_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn direct_messages_are_not_guild_channel_reports() {
    let database = start_database().await;
    let w = world(&database, None).await;
    let (status, dm) = call(
        &w.app,
        Method::POST,
        "/api/v1/users/@me/channels",
        &w.owner,
        Some(json!({"recipient_ids": [w.ann_id]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = json!({"target": "channel", "channel_id": dm["id"], "category": "other"});
    assert_eq!(w.report(&w.ann, body).await.0, StatusCode::FORBIDDEN);
    assert!(w.queue("").await["reports"].as_array().unwrap().is_empty());
}
