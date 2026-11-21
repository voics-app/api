//! The transactional unit of work: applies a write set and enqueues its
//! events in the outbox within the same transaction (P6).

mod applications;
mod authorizations;
mod devices;
mod guilds;
mod identity;

use async_trait::async_trait;
use chrono::Utc;
use sqlx::PgPool;

use application::events::DomainEvent;
use application::ports::UnitOfWork;
use application::write_set::{Mutation, WriteSet};
use application::AppResult;

use super::error::db;
use super::outbox;
use crate::events::route::route;

/// Commits write sets to Postgres, with outbox rows for every emitted event.
#[derive(Clone)]
pub struct PgUnitOfWork {
    pool: PgPool,
    default_region: String,
}

impl PgUnitOfWork {
    pub fn new(pool: PgPool, default_region: String) -> Self {
        Self {
            pool,
            default_region,
        }
    }
}

#[async_trait]
impl UnitOfWork for PgUnitOfWork {
    async fn commit(&self, writes: WriteSet) -> AppResult<()> {
        if writes.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await.map_err(db)?;
        let conn = &mut *tx;
        let mut conditional_events = Vec::new();
        let mut membership_events = Vec::new();
        if let Some((id, token)) = &writes.forum_guard {
            let valid: Option<i64> = sqlx::query_scalar("SELECT id FROM channels WHERE id = $1 AND forum_settings->>'pending' = 'true' AND forum_settings->>'status' = 'publishing' AND forum_settings->>'lease_token' = $2 FOR UPDATE")
                .bind(id.as_i64()).bind(token).fetch_optional(&mut *conn).await.map_err(db)?;
            if valid.is_none() {
                return Err(application::AppError::Conflict {
                    code: 0,
                    message: "Forum publication lease was superseded.".into(),
                });
            }
        }
        for mutation in &writes.mutations {
            if let Mutation::SetScheduledEventInterest {
                guild_id,
                event_id,
                user_id,
                channel_id,
                interested,
            } = mutation
            {
                if super::scheduled_events::set_interest(
                    conn,
                    *guild_id,
                    *event_id,
                    *user_id,
                    *interested,
                )
                .await?
                {
                    conditional_events.push(DomainEvent::ScheduledEventInterest {
                        guild_id: *guild_id,
                        event_id: *event_id,
                        user_id: *user_id,
                        channel_id: *channel_id,
                        interested: *interested,
                    });
                }
                continue;
            }
            if !matches!(mutation, Mutation::InsertAudit(_)) {
                let removed_threads: Vec<i64> = if let Mutation::DeleteMember {
                    guild_id,
                    user_id,
                } = mutation
                {
                    sqlx::query_scalar("SELECT c.id FROM channels c JOIN thread_members tm ON tm.thread_id = c.id WHERE tm.guild_id = $1 AND tm.user_id = $2 ORDER BY c.id FOR UPDATE OF c")
                        .bind(guild_id.as_i64())
                        .bind(user_id.as_i64())
                        .fetch_all(&mut *conn)
                        .await
                        .map_err(db)?
                } else {
                    Vec::new()
                };
                apply(conn, mutation).await?;
                if let Mutation::DeleteMember { guild_id, user_id } = mutation {
                    for id in removed_threads {
                        sqlx::query("UPDATE channels SET thread_membership_version = thread_membership_version + 1 WHERE id = $1")
                            .bind(id)
                            .execute(&mut *conn)
                            .await
                            .map_err(db)?;
                        membership_events.push(DomainEvent::ThreadMembersUpdate {
                            guild_id: *guild_id,
                            thread_id: domain::Snowflake::new(id as u64),
                            user_id: *user_id,
                            joined_at: None,
                            member_count: 0,
                            version: 0,
                            member_ids: Vec::new(),
                        });
                    }
                }
            }
        }
        let mut recorded = std::collections::HashSet::new();
        for mutation in &writes.mutations {
            if let Mutation::InsertAudit(entry) = mutation {
                if super::audit_policy::records(conn, entry).await? {
                    guilds::insert_audit(conn, entry).await?;
                    recorded.insert(entry.id);
                }
            }
        }
        let now = Utc::now();
        for event in writes
            .events
            .iter()
            .chain(&membership_events)
            .chain(&conditional_events)
        {
            if matches!(event, DomainEvent::AuditLogEntryCreate(entry) if !recorded.contains(&entry.id))
            {
                continue;
            }
            // A profile snapshot may predate another factor change. Publish the
            // committed user, including derived MFA state, under the write lock.
            let committed;
            let event = if let DomainEvent::UserUpdate(user) = event {
                let row: super::users::UserRow = sqlx::query_as(&format!(
                    "SELECT {} FROM users WHERE id = $1",
                    super::users::USER_COLUMNS
                ))
                .bind(user.id.as_i64())
                .fetch_one(&mut *conn)
                .await
                .map_err(db)?;
                committed = DomainEvent::UserUpdate(row.into());
                &committed
            } else if let DomainEvent::MemberUpdate(profile) = event {
                committed = DomainEvent::MemberUpdate(
                    super::guilds::committed_member(conn, &profile.member).await?,
                );
                &committed
            } else if let DomainEvent::RoleUpdate(role) = event {
                let row: Option<super::guilds::RoleRow> = sqlx::query_as(&format!(
                    "SELECT {} FROM roles WHERE id = $1",
                    super::guilds::ROLE_COLUMNS
                ))
                .bind(role.id.as_i64())
                .fetch_optional(&mut *conn)
                .await
                .map_err(db)?;
                let Some(row) = row else { continue };
                committed = DomainEvent::RoleUpdate(row.into());
                &committed
            } else if let DomainEvent::ThreadMembersUpdate {
                guild_id,
                thread_id,
                user_id,
                joined_at,
                ..
            } = event
            {
                let (count,): (i64,) =
                    sqlx::query_as("SELECT count(*) FROM thread_members WHERE thread_id = $1")
                        .bind(thread_id.as_i64())
                        .fetch_one(&mut *conn)
                        .await
                        .map_err(db)?;
                let (version,): (i64,) =
                    sqlx::query_as("SELECT thread_membership_version FROM channels WHERE id = $1")
                        .bind(thread_id.as_i64())
                        .fetch_one(&mut *conn)
                        .await
                        .map_err(db)?;
                let ids: Vec<i64> = sqlx::query_scalar(
                    "SELECT user_id FROM thread_members WHERE thread_id = $1 ORDER BY user_id",
                )
                .bind(thread_id.as_i64())
                .fetch_all(&mut *conn)
                .await
                .map_err(db)?;
                committed = DomainEvent::ThreadMembersUpdate {
                    guild_id: *guild_id,
                    thread_id: *thread_id,
                    user_id: *user_id,
                    joined_at: *joined_at,
                    member_count: count as usize,
                    version: version as u64,
                    member_ids: ids
                        .into_iter()
                        .map(|id| domain::Snowflake::new(id as u64))
                        .collect(),
                };
                &committed
            } else {
                event
            };
            for routed in route(event, &self.default_region, now) {
                outbox::enqueue(&mut *conn, &routed, now).await?;
            }
        }
        tx.commit().await.map_err(db)
    }
}

async fn apply(conn: &mut sqlx::PgConnection, mutation: &Mutation) -> AppResult<()> {
    match mutation {
        Mutation::UpdateScheduledEvent {
            event,
            expected_revision,
        } => super::scheduled_events::update(conn, event, *expected_revision).await,
        Mutation::DeleteScheduledEvent {
            guild_id,
            event_id,
            expected_revision,
        } => super::scheduled_events::delete(conn, *guild_id, *event_id, *expected_revision).await,
        Mutation::SetScheduledEventInterest { .. } => unreachable!("interest handled in commit"),
        Mutation::InsertScheduledEvent(event) => super::scheduled_events::insert(conn, event).await,
        Mutation::InsertUser {
            user,
            credentials,
            settings,
        } => {
            identity::upsert_user(&mut *conn, user).await?;
            if let Some(credentials) = credentials {
                identity::insert_credentials(&mut *conn, credentials).await?;
            }
            identity::upsert_settings(conn, settings.as_ref()).await
        }
        Mutation::UpdateUser(user) => identity::upsert_user(conn, user).await,
        Mutation::SetUserAppearance { user_id, patch } => {
            super::profile::set_user(conn, *user_id, patch).await
        }
        Mutation::SetMemberProfile {
            guild_id,
            user_id,
            patch,
        } => super::profile::set_member(conn, *guild_id, *user_id, patch).await,
        Mutation::SetTotpSecret { user_id, secret } => {
            identity::set_totp_secret(conn, *user_id, secret.as_deref()).await
        }
        Mutation::SetPasswordHash { user_id, hash } => {
            identity::set_password_hash(conn, *user_id, hash).await
        }
        Mutation::SetPendingTotp { user_id, secret } => {
            identity::set_pending_totp(conn, *user_id, secret.as_deref()).await
        }
        Mutation::ReplaceBackupCodes { user_id, hashes } => {
            identity::replace_backup_codes(conn, *user_id, hashes).await
        }
        Mutation::SetAccountState { user_id, state } => {
            identity::set_account_state(conn, *user_id, state).await
        }
        Mutation::AnonymizeUser { user_id, username } => {
            identity::anonymize_user(conn, *user_id, username).await
        }
        Mutation::SuspendUser {
            user_id,
            suspension,
        } => identity::suspend_user(conn, *user_id, suspension).await,
        Mutation::UpsertSettings(settings) => identity::upsert_settings(conn, settings).await,
        Mutation::UpsertGuildNotificationSettings(settings) => {
            identity::upsert_guild_notification_settings(conn, settings).await
        }
        Mutation::SetUserNote {
            owner_id,
            target_id,
            note,
        } => identity::set_user_note(conn, *owner_id, *target_id, note.as_deref()).await,
        Mutation::SetRelationship {
            user_id,
            target_id,
            kind,
            since,
        } => identity::set_relationship(conn, *user_id, *target_id, *kind, *since).await,
        Mutation::InsertStageInstance(stage) => {
            sqlx::query("INSERT INTO stage_instances (id, guild_id, channel_id, topic) VALUES ($1, $2, $3, $4)")
                .bind(stage.id.as_i64()).bind(stage.guild_id.as_i64()).bind(stage.channel_id.as_i64())
                .bind(&stage.topic).execute(conn).await.map_err(db)?;
            Ok(())
        }
        Mutation::UpdateStageInstance(stage) => {
            let result = sqlx::query(
                "UPDATE stage_instances SET topic = $1 WHERE channel_id = $2 AND id = $3",
            )
            .bind(&stage.topic)
            .bind(stage.channel_id.as_i64())
            .bind(stage.id.as_i64())
            .execute(conn)
            .await
            .map_err(db)?;
            if result.rows_affected() == 0 {
                return Err(application::AppError::NotFound(
                    application::Resource::Channel,
                ));
            }
            Ok(())
        }
        Mutation::DeleteStageInstance { channel_id, id } => {
            let result =
                sqlx::query("DELETE FROM stage_instances WHERE channel_id = $1 AND id = $2")
                    .bind(channel_id.as_i64())
                    .bind(id.as_i64())
                    .execute(conn)
                    .await
                    .map_err(db)?;
            if result.rows_affected() == 0 {
                return Err(application::AppError::NotFound(
                    application::Resource::Channel,
                ));
            }
            Ok(())
        }
        Mutation::InsertChannel(channel) | Mutation::UpdateChannel(channel) => {
            identity::upsert_channel(conn, channel).await
        }
        Mutation::DeleteChannel(channel_id) => identity::delete_channel(conn, *channel_id).await,
        Mutation::SetThreadMember {
            thread_id,
            guild_id,
            user_id,
            joined_at,
        } => {
            sqlx::query("SELECT id FROM channels WHERE id = $1 FOR UPDATE")
                .bind(thread_id.as_i64())
                .execute(&mut *conn)
                .await
                .map_err(db)?;
            match joined_at {
                Some(joined_at) => {
                    sqlx::query("INSERT INTO thread_members (thread_id, guild_id, user_id, joined_at) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING")
                        .bind(thread_id.as_i64())
                        .bind(guild_id.as_i64())
                        .bind(user_id.as_i64())
                        .bind(joined_at)
                        .execute(&mut *conn)
                        .await
                        .map_err(db)?;
                }
                None => {
                    sqlx::query("DELETE FROM thread_members WHERE thread_id = $1 AND guild_id = $2 AND user_id = $3")
                        .bind(thread_id.as_i64())
                        .bind(guild_id.as_i64())
                        .bind(user_id.as_i64())
                        .execute(&mut *conn)
                        .await
                        .map_err(db)?;
                }
            }
            sqlx::query("UPDATE channels SET thread_membership_version = thread_membership_version + 1 WHERE id = $1")
                .bind(thread_id.as_i64())
                .execute(conn)
                .await
                .map_err(db)?;
            Ok(())
        }
        Mutation::SetDeliveryOverride {
            user_id,
            channel_id,
            mode,
        } => identity::set_delivery_override(conn, *user_id, *channel_id, *mode).await,

        Mutation::InsertGuild(guild) | Mutation::UpdateGuild(guild) => {
            guilds::upsert_guild(conn, guild).await
        }
        Mutation::ApplyOnboarding {
            guild_id,
            user_id,
            revision,
            role_ids,
        } => {
            // Channel overwrite writers take this same guild lock. Inspecting
            // them under it prevents a role from granting moderation via an
            // overwrite while a member selects it.
            super::community::lock(conn, *guild_id).await?;
            let (current,): (i64,) = sqlx::query_as("SELECT onboarding_revision FROM guild_member_screening WHERE guild_id = $1 FOR SHARE")
                .bind(guild_id.as_i64()).fetch_one(&mut *conn).await.map_err(db)?;
            if current != *revision {
                return Err(application::AppError::Conflict {
                    code: 40001,
                    message: "Onboarding changed; review it again.".into(),
                });
            }
            let (pending,): (bool,) = sqlx::query_as(
                "SELECT pending FROM members WHERE guild_id = $1 AND user_id = $2 FOR UPDATE",
            )
            .bind(guild_id.as_i64())
            .bind(user_id.as_i64())
            .fetch_one(&mut *conn)
            .await
            .map_err(db)?;
            if pending {
                return Err(application::AppError::missing_permissions());
            }
            let result = sqlx::query("INSERT INTO guild_onboarding_completions (guild_id, user_id, revision) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
                .bind(guild_id.as_i64()).bind(user_id.as_i64()).bind(revision).execute(&mut *conn).await.map_err(db)?;
            if result.rows_affected() != 1 {
                return Err(application::AppError::Conflict {
                    code: 40001,
                    message: "Onboarding has already been completed.".into(),
                });
            }
            for role_id in role_ids {
                let row: Option<(i64, bool)> = sqlx::query_as("SELECT permissions, managed FROM roles WHERE guild_id = $1 AND id = $2 FOR SHARE")
                    .bind(guild_id.as_i64()).bind(role_id.as_i64()).fetch_optional(&mut *conn).await.map_err(db)?;
                let Some((permissions, managed)) = row else {
                    return Err(application::AppError::Conflict {
                        code: 40001,
                        message: "An onboarding role no longer exists.".into(),
                    });
                };
                if managed
                    || *role_id == *guild_id
                    || (permissions as u64
                        & !application::use_cases::guilds::onboarding::SAFE_ROLE_BITS)
                        != 0
                {
                    return Err(application::AppError::Conflict {
                        code: 40001,
                        message: "An onboarding role is no longer eligible.".into(),
                    });
                }
                let unsafe_overwrite: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM permission_overwrites p JOIN channels c ON c.id = p.channel_id WHERE c.guild_id = $1 AND p.kind = 0 AND p.target_id = $2 AND (p.allow_bits & $3) <> 0)")
                    .bind(guild_id.as_i64()).bind(role_id.as_i64()).bind(!(application::use_cases::guilds::onboarding::SAFE_ROLE_BITS as i64)).fetch_one(&mut *conn).await.map_err(db)?;
                if unsafe_overwrite {
                    return Err(application::AppError::Conflict {
                        code: 40001,
                        message: "An onboarding role has a privileged channel overwrite.".into(),
                    });
                }
                sqlx::query("INSERT INTO member_roles (guild_id, user_id, role_id) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
                    .bind(guild_id.as_i64()).bind(user_id.as_i64()).bind(role_id.as_i64()).execute(&mut *conn).await.map_err(db)?;
            }
            Ok(())
        }
        Mutation::CheckScreeningRevision { guild_id, revision } => {
            let current: (i64,) = sqlx::query_as(
                "SELECT revision FROM guild_member_screening WHERE guild_id = $1 FOR SHARE",
            )
            .bind(guild_id.as_i64())
            .fetch_one(conn)
            .await
            .map_err(db)?;
            if current.0 != *revision {
                return Err(application::AppError::Conflict {
                    code: 40001,
                    message: "Rules changed; review them again.".into(),
                });
            }
            Ok(())
        }
        Mutation::AcceptScreening { guild_id, user_id } => {
            let result = sqlx::query("UPDATE members SET pending = false WHERE guild_id = $1 AND user_id = $2 AND pending = true")
                .bind(guild_id.as_i64()).bind(user_id.as_i64()).execute(conn).await.map_err(db)?;
            if result.rows_affected() != 1 {
                return Err(application::AppError::Conflict {
                    code: 40001,
                    message: "Membership changed; reload and try again.".into(),
                });
            }
            Ok(())
        }
        Mutation::DeleteGuild(guild_id) => guilds::delete_guild(conn, *guild_id).await,
        Mutation::InsertRole(role) | Mutation::UpdateRole(role) => {
            guilds::upsert_role(conn, role).await
        }
        Mutation::ShiftRolesForBot { guild_id } => {
            sqlx::query(
                "UPDATE roles SET position = position + 1 WHERE guild_id = $1 AND id <> $1",
            )
            .bind(guild_id.as_i64())
            .execute(conn)
            .await
            .map_err(db)?;
            Ok(())
        }
        Mutation::DeleteRole { guild_id, role_id } => {
            guilds::delete_role(conn, *guild_id, *role_id).await
        }
        Mutation::UpsertMember(member) => guilds::upsert_member(conn, member).await,
        Mutation::InsertMember(member) => guilds::insert_member(conn, member).await,
        Mutation::DeleteMember { guild_id, user_id } => {
            guilds::delete_member(conn, *guild_id, *user_id).await
        }
        Mutation::InsertBan {
            guild_id,
            user_id,
            reason,
        } => guilds::insert_ban(conn, *guild_id, *user_id, reason.as_deref()).await,
        Mutation::ResolveBanAppeal {
            guild_id,
            appeal_id,
            user_id,
            reviewer_id,
            accept,
        } => {
            let ban_time: Option<chrono::DateTime<Utc>> = sqlx::query_scalar(
                "SELECT created_at FROM bans WHERE guild_id=$1 AND user_id=$2 FOR UPDATE",
            )
            .bind(guild_id.as_i64())
            .bind(user_id.as_i64())
            .fetch_optional(&mut *conn)
            .await
            .map_err(db)?;
            let updated = sqlx::query("UPDATE ban_appeals SET status=$4, resolved_at=now(), resolved_by=$5 WHERE id=$1 AND guild_id=$2 AND user_id=$3 AND status='open' AND ban_created_at=$6")
                .bind(appeal_id.as_i64()).bind(guild_id.as_i64()).bind(user_id.as_i64())
                .bind(if *accept { "accepted" } else { "denied" }).bind(reviewer_id.as_i64()).bind(ban_time)
                .execute(&mut *conn).await.map_err(db)?;
            if updated.rows_affected() != 1 {
                return Err(application::AppError::Conflict {
                    code: 0,
                    message: "Appeal is no longer open for the current ban".into(),
                });
            }
            if *accept {
                guilds::delete_ban(conn, *guild_id, *user_id).await?;
            }
            Ok(())
        }
        Mutation::DeleteBan { guild_id, user_id } => {
            guilds::delete_ban(conn, *guild_id, *user_id).await
        }
        Mutation::InsertInvite(invite) => guilds::insert_invite(conn, invite).await,
        Mutation::DeleteInvite(code) => guilds::delete_invite(conn, code).await,
        Mutation::ConsumeInvite(code) => guilds::consume_invite(conn, code).await,
        Mutation::SetVanityInvite { guild_id, invite } => {
            super::vanity::set(conn, *guild_id, invite.as_ref()).await
        }
        Mutation::ConsumeVanityInvite { guild_id, code } => {
            super::vanity::consume(conn, *guild_id, code).await
        }
        Mutation::InsertEmoji(emoji) | Mutation::UpdateEmoji(emoji) => {
            guilds::upsert_emoji(conn, emoji).await
        }
        Mutation::DeleteEmoji { guild_id, emoji_id } => {
            guilds::delete_emoji(conn, *guild_id, *emoji_id).await
        }
        Mutation::InsertAudit(entry) => guilds::insert_audit(conn, entry).await,
        Mutation::SetAuditPolicy { guild_id, policy } => {
            super::audit_policy::set(conn, *guild_id, *policy).await
        }
        Mutation::SetCommunity { guild_id, channels } => {
            super::community::set(conn, *guild_id, *channels).await
        }

        Mutation::InsertApplication(app) | Mutation::UpdateApplication(app) => {
            applications::upsert_application(conn, app).await
        }
        Mutation::SetBotToken {
            application_id,
            bot_user_id,
            token_hash,
        } => applications::set_bot_token(conn, *application_id, *bot_user_id, token_hash).await,
        Mutation::UpsertCommand(command) => applications::upsert_command(conn, command).await,
        Mutation::DeleteCommand {
            application_id,
            command_id,
        } => applications::delete_command(conn, *application_id, *command_id).await,
        Mutation::ReplaceCommands {
            application_id,
            guild_id,
            commands,
        } => applications::replace_commands(conn, *application_id, *guild_id, commands).await,
        Mutation::InsertDevice(device) => devices::insert_device(conn, device).await,
        Mutation::DeleteDevice { user_id, device_id } => {
            devices::delete_device(conn, *user_id, *device_id).await
        }

        Mutation::InsertInteraction(interaction) => {
            applications::insert_interaction(conn, interaction).await
        }
        Mutation::UpdateInteraction {
            id,
            state,
            original_message_id,
        } => applications::update_interaction(conn, *id, *state, *original_message_id).await,
        Mutation::MarkAnnouncementPublished {
            source_channel_id,
            source_message_id,
        } => {
            sqlx::query("UPDATE announcement_publications SET marked=true WHERE source_channel_id=$1 AND source_message_id=$2")
                .bind(source_channel_id.as_i64()).bind(source_message_id.as_i64()).execute(conn).await.map_err(db)?;
            Ok(())
        }
        Mutation::EnqueueAnnouncement {
            source_channel_id,
            source_message_id,
            payload,
            deliveries,
        } => {
            sqlx::query("INSERT INTO announcement_publications (source_channel_id, source_message_id, payload) VALUES ($1, $2, $3)")
                .bind(source_channel_id.as_i64()).bind(source_message_id.as_i64()).bind(payload)
                .execute(&mut *conn).await.map_err(db)?;
            for (target, webhook, destination_id) in deliveries {
                sqlx::query("INSERT INTO announcement_deliveries (source_channel_id, source_message_id, target_channel_id, webhook_id, destination_message_id) SELECT $1, $2, $3, $4, $5 FROM announcement_followers WHERE source_channel_id=$1 AND target_channel_id=$3 AND webhook_id=$4 FOR KEY SHARE")
                    .bind(source_channel_id.as_i64()).bind(source_message_id.as_i64()).bind(target.as_i64())
                    .bind(webhook.as_i64()).bind(destination_id.as_i64()).execute(&mut *conn).await.map_err(db)?;
            }
            Ok(())
        }
        Mutation::InsertWebhook(webhook) => applications::insert_webhook(conn, webhook).await,
        Mutation::InsertAnnouncementFollower {
            source_channel_id,
            target_channel_id,
            webhook_id,
        } => {
            sqlx::query("INSERT INTO announcement_followers (source_channel_id, target_channel_id, webhook_id) VALUES ($1, $2, $3)")
                .bind(source_channel_id.as_i64())
                .bind(target_channel_id.as_i64())
                .bind(webhook_id.as_i64())
                .execute(conn)
                .await
                .map_err(db)?;
            Ok(())
        }
        Mutation::GuardApplicationInstallation {
            guild_id,
            application_id,
            bot_user_id,
            expected_generation,
            expected_revision,
        } => {
            authorizations::guard(
                conn,
                *guild_id,
                *application_id,
                *bot_user_id,
                *expected_generation,
                *expected_revision,
            )
            .await
        }
        Mutation::RegisterApplicationInstallation {
            guild_id,
            application_id,
            bot_user_id,
            installed_at,
        } => {
            authorizations::register(
                conn,
                *guild_id,
                *application_id,
                *bot_user_id,
                *installed_at,
            )
            .await
        }
        Mutation::RecordBotConsent(consent) => authorizations::consent(conn, consent).await,
        Mutation::RevokeBotConsent { id, user_id } => {
            authorizations::revoke(conn, *id, *user_id).await
        }
        Mutation::UpdateWebhook { id, patch } => {
            applications::update_webhook(conn, *id, patch).await
        }
        Mutation::SetWebhookToken { id, token_hash } => {
            applications::set_webhook_token(conn, *id, token_hash).await
        }
        Mutation::DeleteWebhook(webhook_id) => {
            applications::delete_webhook(conn, *webhook_id).await
        }
    }
}
