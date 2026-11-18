use super::*;

#[tokio::test]
async fn restricted_push_honours_account_hide_and_never_enables_background_previews() {
    let database = start_database().await;
    let app = app(&database);
    let (owner, owner_id) = register(&app, "owner").await;
    let (member, member_id) = register(&app, "member").await;
    let (_, guild) = call(
        &app,
        Method::POST,
        "/api/v1/guilds",
        &owner,
        Some(json!({"name": "Test"})),
    )
    .await;
    let guild_id = guild["id"].as_str().unwrap();
    let (_, channels) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        None,
    )
    .await;
    let channel = channels
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["type"] == 0)
        .expect("default guild text channel")["id"]
        .as_str()
        .unwrap();
    let (_, invite) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{channel}/invites"),
        &owner,
        Some(json!({})),
    )
    .await;
    let code = invite["code"].as_str().unwrap();
    assert_eq!(
        call(
            &app,
            Method::POST,
            &format!("/api/v1/invites/{code}"),
            &member,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        put_token(&app, &member, TOKEN_B).await,
        StatusCode::NO_CONTENT
    );

    // Sparse default: ordinary content still honours the registered preview preference.
    assert_eq!(
        targets(&app, channel, &owner_id).await["targets"][0]["tokens"][0]["previews"],
        true
    );
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &format!("/api/v1/channels/{channel}"),
            &owner,
            Some(json!({"nsfw": true}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        targets(&app, channel, &owner_id).await["targets"][0]["tokens"][0]["previews"],
        false
    );
    let (status, thread) = call(
        &app,
        Method::POST,
        &format!("/api/v1/channels/{channel}/threads"),
        &owner,
        Some(json!({"name": "Inherited restriction", "type": 11})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{thread}");
    let thread_id = thread["id"].as_str().unwrap();
    assert_eq!(
        targets(&app, thread_id, &owner_id).await["targets"][0]["tokens"][0]["previews"],
        false
    );

    // Fixture seeds persisted preferences independently of the separate preference-validation companion.
    for (hide, expected_targets) in [(true, 0), (false, 1)] {
        sqlx::query("UPDATE user_settings SET preferences = jsonb_build_object('hide_age_restricted_channels', $1::boolean) WHERE user_id = $2")
            .bind(hide).bind(member_id.parse::<i64>().unwrap()).execute(&database.pool).await.unwrap();
        let found = targets(&app, channel, &owner_id).await;
        assert_eq!(found["targets"].as_array().unwrap().len(), expected_targets);
        assert_eq!(
            targets(&app, thread_id, &owner_id).await["targets"]
                .as_array()
                .unwrap()
                .len(),
            expected_targets
        );
        if !hide {
            assert_eq!(found["targets"][0]["tokens"][0]["previews"], false);
        }
    }

    // An unrestricted child still inherits its category's restriction.
    let (status, category) = call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{guild_id}/channels"),
        &owner,
        Some(json!({"name": "Restricted", "type": 4, "nsfw": true})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let category_id = category["id"].as_str().unwrap();
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &format!("/api/v1/channels/{channel}"),
            &owner,
            Some(json!({"nsfw": false, "parent_id": category_id}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        targets(&app, channel, &owner_id).await["targets"][0]["tokens"][0]["previews"],
        false
    );
    sqlx::query("UPDATE user_settings SET preferences = '{\"hide_age_restricted_channels\":true}'::jsonb WHERE user_id = $1")
        .bind(member_id.parse::<i64>().unwrap()).execute(&database.pool).await.unwrap();
    assert!(targets(&app, channel, &owner_id).await["targets"]
        .as_array()
        .unwrap()
        .is_empty());

    // Removing the restriction restores the existing token preference even when Hide stays on.
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &format!("/api/v1/channels/{category_id}"),
            &owner,
            Some(json!({"nsfw": false}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        targets(&app, channel, &owner_id).await["targets"][0]["tokens"][0]["previews"],
        true
    );
    assert!(
        stored(&database).await[0].3,
        "the per-channel policy never rewrites the device token"
    );
}
