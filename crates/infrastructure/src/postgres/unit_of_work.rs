//! The transactional unit of work: applies a write set and enqueues its
//! events in the outbox within the same transaction (P6).

mod applications;
mod devices;
mod guilds;
mod identity;

use async_trait::async_trait;
use chrono::Utc;
use sqlx::PgPool;

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
        for mutation in &writes.mutations {
            apply(conn, mutation).await?;
        }
        let now = Utc::now();
        for event in &writes.events {
            for routed in route(event, &self.default_region, now) {
                outbox::enqueue(&mut *conn, &routed, now).await?;
            }
        }
        tx.commit().await.map_err(db)
    }
}

async fn apply(conn: &mut sqlx::PgConnection, mutation: &Mutation) -> AppResult<()> {
    match mutation {
        Mutation::InsertUser {
            user,
            credentials,
            settings,
        } => {
            identity::upsert_user(&mut *conn, user).await?;
            if let Some(credentials) = credentials {
                identity::insert_credentials(&mut *conn, credentials).await?;
            }
            identity::upsert_settings(conn, settings).await
        }
        Mutation::UpdateUser(user) => identity::upsert_user(conn, user).await,
        Mutation::SetTotpSecret { user_id, secret } => {
            identity::set_totp_secret(conn, *user_id, secret.as_deref()).await
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
        Mutation::InsertChannel(channel) | Mutation::UpdateChannel(channel) => {
            identity::upsert_channel(conn, channel).await
        }
        Mutation::DeleteChannel(channel_id) => identity::delete_channel(conn, *channel_id).await,
        Mutation::SetDeliveryOverride {
            user_id,
            channel_id,
            mode,
        } => identity::set_delivery_override(conn, *user_id, *channel_id, *mode).await,

        Mutation::InsertGuild(guild) | Mutation::UpdateGuild(guild) => {
            guilds::upsert_guild(conn, guild).await
        }
        Mutation::DeleteGuild(guild_id) => guilds::delete_guild(conn, *guild_id).await,
        Mutation::InsertRole(role) | Mutation::UpdateRole(role) => {
            guilds::upsert_role(conn, role).await
        }
        Mutation::DeleteRole { guild_id, role_id } => {
            guilds::delete_role(conn, *guild_id, *role_id).await
        }
        Mutation::UpsertMember(member) => guilds::upsert_member(conn, member).await,
        Mutation::DeleteMember { guild_id, user_id } => {
            guilds::delete_member(conn, *guild_id, *user_id).await
        }
        Mutation::InsertBan {
            guild_id,
            user_id,
            reason,
        } => guilds::insert_ban(conn, *guild_id, *user_id, reason.as_deref()).await,
        Mutation::DeleteBan { guild_id, user_id } => {
            guilds::delete_ban(conn, *guild_id, *user_id).await
        }
        Mutation::InsertInvite(invite) => guilds::insert_invite(conn, invite).await,
        Mutation::DeleteInvite(code) => guilds::delete_invite(conn, code).await,
        Mutation::ConsumeInvite(code) => guilds::consume_invite(conn, code).await,
        Mutation::InsertEmoji(emoji) | Mutation::UpdateEmoji(emoji) => {
            guilds::upsert_emoji(conn, emoji).await
        }
        Mutation::DeleteEmoji { guild_id, emoji_id } => {
            guilds::delete_emoji(conn, *guild_id, *emoji_id).await
        }
        Mutation::InsertAudit(entry) => guilds::insert_audit(conn, entry).await,

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
        Mutation::InsertWebhook(webhook) | Mutation::UpdateWebhook(webhook) => {
            applications::upsert_webhook(conn, webhook).await
        }
        Mutation::DeleteWebhook(webhook_id) => {
            applications::delete_webhook(conn, *webhook_id).await
        }
    }
}
