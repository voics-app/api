use super::*;

#[tokio::test]
async fn an_installed_preview_cannot_reinstall_a_bot_removed_before_the_post() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = router(ports(&database.pool, &public, &private));
    let owner = register(&app, "previewmanager").await;
    let (guild, _) = guild(&app, &owner).await;
    let (application, _) = application(&app, &owner).await;
    assert_eq!(
        authorize(&app, &owner, &application, &guild, "2048")
            .await
            .0,
        StatusCode::OK
    );
    let preview_path = format!("/api/v1/oauth2/authorize?client_id={application}&scope=bot+applications.commands&permissions=2048");
    let (status, preview) = call(&app, Method::GET, &preview_path, &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    let reviewed = preview["guilds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["id"] == guild)
        .unwrap();
    let reviewed_permissions = reviewed["bot_permissions"]
        .as_str()
        .expect("the UI presents Record consent for an existing installation")
        .to_owned();
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/v1/guilds/{guild}/integrations/{application}"),
            &owner,
            None,
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    // The removal has committed before POST begins. A guard that snapshots only
    // inside POST cannot identify the installation the user actually reviewed.
    assert_eq!(
        authorize_revision(
            &app,
            &owner,
            &application,
            &guild,
            &reviewed_permissions,
            reviewed["installation_revision"].as_str().unwrap(),
        )
            .await
            .0,
        StatusCode::CONFLICT,
        "Record consent must not reinstall a bot removed since its preview"
    );
    let (status, installed) = call(
        &app,
        Method::GET,
        &format!("/api/v1/guilds/{guild}/integrations"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(installed.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn consent_cannot_acknowledge_permissions_changed_after_the_preview() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = router(ports(&database.pool, &public, &private));
    let owner = register(&app, "manager").await;
    let (guild, _) = guild(&app, &owner).await;
    let (application, bot) = application(&app, &owner).await;
    assert_eq!(
        authorize(&app, &owner, &application, &guild, "2048")
            .await
            .0,
        StatusCode::OK
    );
    let before = call(&app, Method::GET, "/api/v1/oauth2/tokens", &owner, None)
        .await
        .1;
    let acknowledged = before[0]["permissions"].as_str().unwrap();
    // Another administrator gives the bot a new role through the ordinary HTTP routes.
    let (status, role) = call(
        &app,
        Method::POST,
        &format!("/api/v1/guilds/{guild}/roles"),
        &owner,
        Some(json!({"name":"Bot administrator", "permissions":"8"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let role = role["id"].as_str().unwrap();
    assert_eq!(
        call(
            &app,
            Method::PUT,
            &format!("/api/v1/guilds/{guild}/members/{bot}/roles/{role}"),
            &owner,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        authorize(&app, &owner, &application, &guild, acknowledged)
            .await
            .0,
        StatusCode::CONFLICT,
        "unseen effective permissions require another review"
    );
    assert_eq!(
        call(&app, Method::GET, "/api/v1/oauth2/tokens", &owner, None)
            .await
            .1,
        before,
        "a stale preview cannot change recorded consent"
    );
    assert_eq!(
        reauthorize_current(&app, &owner, &application, &guild)
            .await
            .0,
        StatusCode::OK
    );
    let after = call(&app, Method::GET, "/api/v1/oauth2/tokens", &owner, None)
        .await
        .1;
    assert_eq!(
        after[0]["permissions"],
        domain::permissions::Permissions::ALL.bits().to_string()
    );
}

#[tokio::test]
async fn old_absent_and_replaced_previews_cannot_target_a_new_installation() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = router(ports(&database.pool, &public, &private));
    let owner = register(&app, "revisionmanager").await;
    let (guild, _) = guild(&app, &owner).await;
    let (application, _) = application(&app, &owner).await;
    let installations = format!("/api/v1/guilds/{guild}/integrations");
    let remove_path = format!("{installations}/{application}");
    let original = authorization_preview(&app, &owner, &application, &guild, "2048").await;
    assert_eq!(original["installation_revision"], "absent:0");
    assert_eq!(authorize(&app, &owner, &application, &guild, "2048").await.0, StatusCode::OK);
    let installed = authorization_preview(&app, &owner, &application, &guild, "2048").await;
    assert_eq!(call(&app, Method::DELETE, &remove_path, &owner, None).await.0, StatusCode::NO_CONTENT);
    let removed = authorization_preview(&app, &owner, &application, &guild, "2048").await;
    assert_ne!(original["installation_revision"], removed["installation_revision"]);
    assert!(removed["bot_permissions"].is_null());
    assert_eq!(
        authorize_revision(&app, &owner, &application, &guild, "2048", original["installation_revision"].as_str().unwrap()).await.0,
        StatusCode::CONFLICT,
        "initial absence cannot undo an intervening installation and removal"
    );
    assert_eq!(call(&app, Method::GET, &installations, &owner, None).await.1, json!([]));
    assert_eq!(call(&app, Method::GET, "/api/v1/oauth2/tokens", &owner, None).await.1, json!([]));
    assert_eq!(authorize(&app, &owner, &application, &guild, "2048").await.0, StatusCode::OK);
    let fresh_grants = call(&app, Method::GET, "/api/v1/oauth2/tokens", &owner, None).await.1;
    assert_eq!(
        authorize_revision(&app, &owner, &application, &guild, installed["bot_permissions"].as_str().unwrap(), installed["installation_revision"].as_str().unwrap()).await.0,
        StatusCode::CONFLICT,
        "equal permissions cannot attach old consent to a replacement installation"
    );
    assert_eq!(call(&app, Method::GET, "/api/v1/oauth2/tokens", &owner, None).await.1, fresh_grants);
    assert_eq!(call(&app, Method::DELETE, &remove_path, &owner, None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(
        authorize_revision(&app, &owner, &application, &guild, "2048", removed["installation_revision"].as_str().unwrap()).await.0,
        StatusCode::CONFLICT,
        "an earlier removal is not the current absent state"
    );
    assert_eq!(call(&app, Method::GET, &installations, &owner, None).await.1, json!([]));
    assert_eq!(authorize(&app, &owner, &application, &guild, "2048").await.0, StatusCode::OK);
}

#[tokio::test]
async fn authorization_requires_a_non_null_preview_revision() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = router(ports(&database.pool, &public, &private));
    let owner = register(&app, "requiredrevision").await;
    let (guild, _) = guild(&app, &owner).await;
    let (application, _) = application(&app, &owner).await;
    let base = json!({"client_id": application, "guild_id": guild, "permissions": "0", "scope": "bot"});
    assert_eq!(call(&app, Method::POST, "/api/v1/oauth2/authorize", &owner, Some(base.clone())).await.0, StatusCode::BAD_REQUEST);
    for invalid in [Value::Null, json!(""), json!("absent:01"), json!("installed:0"), json!("installed:9223372036854775808")] {
        let mut body = base.clone();
        body["installation_revision"] = invalid;
        assert_eq!(call(&app, Method::POST, "/api/v1/oauth2/authorize", &owner, Some(body)).await.0, StatusCode::BAD_REQUEST);
    }
    assert_eq!(call(&app, Method::GET, "/api/v1/oauth2/tokens", &owner, None).await.1, json!([]));
    assert_eq!(call(&app, Method::GET, &format!("/api/v1/guilds/{guild}/integrations"), &owner, None).await.1, json!([]));
}

#[tokio::test]
async fn a_delayed_first_install_cannot_undo_an_intervening_install_and_admin_removal() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let mut ports = ports(&database.pool, &public, &private);
    let paused = Arc::new(PauseCommit {
        inner: ports.unit_of_work.clone(),
        matches: |m| matches!(m, Mutation::RecordBotConsent(_)),
        armed: AtomicBool::new(false),
        reached: Notify::new(),
        release: Notify::new(),
    });
    ports.unit_of_work = paused.clone();
    let app = router(ports);
    let owner = register(&app, "manager").await;
    let (guild, _) = guild(&app, &owner).await;
    let (application, _) = application(&app, &owner).await;
    paused.armed.store(true, Ordering::SeqCst);
    let stale = {
        let app = app.clone();
        let owner = owner.clone();
        let application = application.clone();
        let guild = guild.clone();
        tokio::spawn(async move { authorize(&app, &owner, &application, &guild, "2048").await })
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        paused.reached.notified(),
    )
    .await
    .unwrap();
    assert_eq!(
        authorize(&app, &owner, &application, &guild, "2048")
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/v1/guilds/{guild}/integrations/{application}"),
            &owner,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    paused.release.notify_one();
    assert_eq!(
        stale.await.unwrap().0,
        StatusCode::CONFLICT,
        "absence after removal is a different lifecycle state than initial absence"
    );
    assert_eq!(
        call(
            &app,
            Method::GET,
            &format!("/api/v1/guilds/{guild}/integrations"),
            &owner,
            None
        )
        .await
        .1,
        json!([])
    );
    assert_eq!(
        call(&app, Method::GET, "/api/v1/oauth2/tokens", &owner, None)
            .await
            .1,
        json!([])
    );
}

#[tokio::test]
async fn stale_consent_cannot_attach_to_a_reinstalled_bot_and_first_installs_do_not_duplicate_roles(
) {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let mut ports = ports(&database.pool, &public, &private);
    let paused = Arc::new(PauseCommit {
        inner: ports.unit_of_work.clone(),
        matches: |m| matches!(m, Mutation::RecordBotConsent(_)),
        armed: AtomicBool::new(false),
        reached: Notify::new(),
        release: Notify::new(),
    });
    ports.unit_of_work = paused.clone();
    let app = router(ports);
    let owner = register(&app, "manager").await;
    let (guild, _) = guild(&app, &owner).await;
    let (application, bot) = application(&app, &owner).await;

    // Both requests read absence. The request that reaches the transaction first wins.
    paused.armed.store(true, Ordering::SeqCst);
    let first = {
        let app = app.clone();
        let owner = owner.clone();
        let application = application.clone();
        let guild = guild.clone();
        tokio::spawn(async move { authorize(&app, &owner, &application, &guild, "2048").await })
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        paused.reached.notified(),
    )
    .await
    .unwrap();
    let second = authorize(&app, &owner, &application, &guild, "2048").await;
    paused.release.notify_one();
    assert_eq!(second.0, StatusCode::OK);
    assert_eq!(first.await.unwrap().0, StatusCode::CONFLICT);
    let grants = call(&app, Method::GET, "/api/v1/oauth2/tokens", &owner, None)
        .await
        .1;
    assert_eq!(grants.as_array().map(Vec::len), Some(1));
    let old_id = grants[0]["id"].clone();

    // This request read generation 1. Removing and explicitly reinstalling creates generation 2.
    paused.armed.store(true, Ordering::SeqCst);
    let stale = {
        let app = app.clone();
        let owner = owner.clone();
        let application = application.clone();
        let guild = guild.clone();
        tokio::spawn(async move { reauthorize_current(&app, &owner, &application, &guild).await })
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        paused.reached.notified(),
    )
    .await
    .unwrap();
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/v1/guilds/{guild}/integrations/{application}"),
            &owner,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        authorize(&app, &owner, &application, &guild, "2048")
            .await
            .0,
        StatusCode::OK
    );
    let fresh = call(&app, Method::GET, "/api/v1/oauth2/tokens", &owner, None)
        .await
        .1;
    paused.release.notify_one();
    assert_eq!(stale.await.unwrap().0, StatusCode::CONFLICT);
    let final_grants = call(&app, Method::GET, "/api/v1/oauth2/tokens", &owner, None)
        .await
        .1;
    assert_eq!(final_grants, fresh);
    assert_ne!(final_grants[0]["id"], old_id);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM roles WHERE guild_id=$1 AND bot_id=$2 AND managed",
    )
    .bind(guild.parse::<i64>().unwrap())
    .bind(bot.parse::<i64>().unwrap())
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn installation_and_consent_roll_back_together_when_the_outbox_fails() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = router(ports(&database.pool, &public, &private));
    let owner = register(&app, "manager").await;
    let (guild, _) = guild(&app, &owner).await;
    let (application, bot) = application(&app, &owner).await;
    sqlx::raw_sql("CREATE FUNCTION reject_test_outbox() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'test outbox failure'; END $$; CREATE TRIGGER reject_test_outbox BEFORE INSERT ON outbox FOR EACH ROW EXECUTE FUNCTION reject_test_outbox();").execute(&database.pool).await.unwrap();
    assert_eq!(
        authorize(&app, &owner, &application, &guild, "2048")
            .await
            .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM members WHERE guild_id=$1 AND user_id=$2")
            .bind(guild.parse::<i64>().unwrap())
            .bind(bot.parse::<i64>().unwrap())
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
    for table in [
        "application_authorizations",
        "guild_application_installations",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&database.pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "{table} is atomic with installation");
    }
    let roles: i64 = sqlx::query_scalar("SELECT count(*) FROM roles WHERE bot_id=$1")
        .bind(bot.parse::<i64>().unwrap())
        .fetch_one(&database.pool)
        .await
        .unwrap();
    assert_eq!(roles, 0);
    sqlx::query("DROP TRIGGER reject_test_outbox ON outbox")
        .execute(&database.pool)
        .await
        .unwrap();
    assert_eq!(
        authorize(&app, &owner, &application, &guild, "2048")
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn scope_private_app_permission_and_personal_revocation_boundaries_are_enforced() {
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = router(ports(&database.pool, &public, &private));
    let owner = register(&app, "owner").await;
    let manager = register(&app, "manager").await;
    let (guild, _) = guild(&app, &owner).await;
    let (application, _) = application(&app, &owner).await;
    let info_path = format!("/api/v1/oauth2/authorize?client_id={application}&scope=bot+applications.commands&permissions=2048");
    let (status, info) = call(&app, Method::GET, &info_path, &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(info["permissions"], "2048");
    assert_eq!(info["scopes"], json!(["bot", "applications.commands"]));
    assert!(info["guilds"][0]["bot_permissions"].is_null());
    for suffix in [
        "&redirect_uri=https://example.invalid",
        "&response_type=code",
    ] {
        assert_eq!(
            call(
                &app,
                Method::GET,
                &format!("{info_path}{suffix}"),
                &owner,
                None
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    for (scope, permissions) in [
        ("identify", "0"),
        ("bot guilds", "0"),
        ("bot", "9223372036854775808"),
    ] {
        assert_eq!(call(&app, Method::POST, "/api/v1/oauth2/authorize", &owner, Some(json!({"client_id":application,"guild_id":guild,"scope":scope,"permissions":permissions,"installation_revision":"absent:0"}))).await.0, StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &format!("/api/v1/applications/{application}"),
            &owner,
            Some(json!({"bot_public":false}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, Method::GET, &info_path, &manager, None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        authorize_revision(&app, &manager, &application, &guild, "0", "absent:0").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &format!("/api/v1/applications/{application}"),
            &owner,
            Some(json!({"bot_public":true}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let user = call(&app, Method::GET, "/api/v1/users/@me", &manager, None)
        .await
        .1["id"]
        .as_str()
        .unwrap()
        .parse::<i64>()
        .unwrap();
    let guild_id = guild.parse::<i64>().unwrap();
    sqlx::query("INSERT INTO members(guild_id,user_id,joined_at) VALUES($1,$2,now())")
        .bind(guild_id)
        .bind(user)
        .execute(&database.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO roles(id,guild_id,name,position,permissions) VALUES(1001,$1,'Manager',10,32)",
    )
    .bind(guild_id)
    .execute(&database.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO member_roles(guild_id,user_id,role_id) VALUES($1,$2,1001)")
        .bind(guild_id)
        .bind(user)
        .execute(&database.pool)
        .await
        .unwrap();
    assert_eq!(
        authorize(&app, &manager, &application, &guild, "8").await.0,
        StatusCode::FORBIDDEN,
        "managers cannot grant administrator"
    );
    assert_eq!(
        authorize(&app, &manager, &application, &guild, "2048")
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/v1/guilds/{guild}/integrations/{application}"),
            &manager,
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN,
        "removal also requires kick permission"
    );
    let grant = call(&app, Method::GET, "/api/v1/oauth2/tokens", &manager, None)
        .await
        .1[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    sqlx::query("UPDATE roles SET permissions=0 WHERE id=1001")
        .execute(&database.pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/v1/oauth2/tokens/{grant}"),
            &manager,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT,
        "personal revocation requires no current management permission"
    );
    assert_eq!(
        call(
            &app,
            Method::GET,
            &format!("/api/v1/guilds/{guild}/integrations"),
            &manager,
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let info = call(&app, Method::GET, &info_path, &owner, None).await.1;
    assert!(info["guilds"][0]["bot_permissions"].is_string());
}
