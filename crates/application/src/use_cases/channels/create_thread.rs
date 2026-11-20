use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use domain::audit::{AuditAction, AuditChange};
use domain::channel::{Channel, ChannelType, ThreadMetadata, AUTO_ARCHIVE_DURATIONS};
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::channel_access::ChannelAccess;
use crate::support::system_messages;
use crate::use_cases::channels::channel_rules::{apply_name, apply_slowmode};
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone, Default)]
pub struct CreateThreadInput {
    pub name: String,
    pub kind: Option<u8>,
    pub auto_archive_duration: Option<u32>,
    pub invitable: Option<bool>,
    pub rate_limit_per_user: Option<u32>,
    pub applied_tags: Vec<String>,
    pub message: Option<Value>,
    pub request_id: Option<String>,
}

/// `POST /channels/{id}/threads`.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    parent_id: Snowflake,
    input: CreateThreadInput,
    audit: AuditContext<'_>,
) -> AppResult<Channel> {
    let access = ChannelAccess::visible(ports, parent_id, actor.user_id).await?;
    let ctx = access.guild()?;
    let parent = &access.channel;
    if !parent.kind.supports_threads() {
        return Err(AppError::invalid(
            "channel_id",
            "CHANNEL_TYPE_INVALID",
            "This channel cannot have threads.",
        ));
    }
    let kind = match (parent.kind, input.kind.unwrap_or(11)) {
        (ChannelType::GuildAnnouncement, _) => ChannelType::AnnouncementThread,
        (_, 11) => ChannelType::PublicThread,
        (_, 12) => ChannelType::PrivateThread,
        _ => {
            return Err(AppError::invalid(
                "type",
                "BASE_TYPE_CHOICES",
                "Thread type must be 11 or 12.",
            ))
        }
    };
    let required = if kind == ChannelType::PrivateThread {
        Permissions::CREATE_PRIVATE_THREADS
    } else {
        Permissions::CREATE_PUBLIC_THREADS
    };
    access.require(required)?;
    if parent.kind == ChannelType::GuildForum {
        if kind != ChannelType::PublicThread {
            return Err(AppError::invalid(
                "type",
                "CHANNEL_TYPE_INVALID",
                "Forum posts must be public.",
            ));
        }
    } else if !input.applied_tags.is_empty() {
        return Err(AppError::invalid(
            "applied_tags",
            "CHANNEL_TYPE_INVALID",
            "Tags require a forum channel.",
        ));
    }
    if parent.kind == ChannelType::GuildForum {
        return create_forum_post(ports, actor, parent, input, audit).await;
    }
    if input.message.is_some() || input.request_id.is_some() {
        return Err(AppError::invalid(
            "message",
            "CHANNEL_TYPE_INVALID",
            "Embedded messages require a forum.",
        ));
    }
    let duration = input.auto_archive_duration.unwrap_or(1440);
    if !AUTO_ARCHIVE_DURATIONS.contains(&duration) {
        return Err(AppError::invalid(
            "auto_archive_duration",
            "BASE_TYPE_CHOICES",
            "Invalid duration.",
        ));
    }
    let now = ports.clock.now();
    let mut thread = Channel::new(ports.ids.next_id()?, kind, parent.guild_id, now);
    apply_name(&mut thread, &input.name)?;
    if let Some(seconds) = input.rate_limit_per_user {
        apply_slowmode(&mut thread, seconds)?;
    }
    thread.applied_tags = input.applied_tags;
    thread.parent_id = Some(parent.id);
    thread.owner_id = Some(actor.user_id);
    thread.nsfw = parent.nsfw;
    if parent.kind == ChannelType::GuildForum {
        thread.delivery_mode = parent.delivery_mode;
    }
    thread.thread_metadata = Some(ThreadMetadata {
        archived: false,
        auto_archive_duration: duration,
        archive_timestamp: now,
        locked: false,
        invitable: input.invitable.unwrap_or(true),
        create_timestamp: now,
    });
    let changes = vec![AuditChange::new("name", None, Some(json!(thread.name)))];
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::InsertChannel(thread.clone()))
        .emit(DomainEvent::ThreadCreate(thread.clone()))
        .audit(audit.entry(
            ports,
            ctx.guild.id,
            AuditAction::ThreadCreate,
            Some(thread.id),
            changes,
        )?);
    ports.unit_of_work.commit(writes).await?;
    system_messages::announce_thread(ports, &thread, actor.user_id).await;
    Ok(thread)
}

/// A forum reservation is bound to its actor and request body. A claimed delivery
/// is never sent twice: ambiguous outcomes are reconciled by its reserved ID.
async fn create_forum_post(
    ports: &Ports,
    actor: &Actor,
    parent: &Channel,
    input: CreateThreadInput,
    audit: AuditContext<'_>,
) -> AppResult<Channel> {
    if parent.delivery_mode != domain::delivery_mode::DeliveryMode::Standard {
        return Err(AppError::invalid(
            "message",
            "CHANNEL_TYPE_INVALID",
            "Private forum creation requires encrypted delivery and is not yet supported.",
        ));
    }
    let access = ChannelAccess::visible(ports, parent.id, actor.user_id).await?;
    access.require(Permissions::SEND_MESSAGES_IN_THREADS)?;
    let request_key = input
        .request_id
        .as_deref()
        .and_then(|v| uuid::Uuid::parse_str(v).ok())
        .ok_or_else(|| {
            AppError::invalid(
                "request_id",
                "BASE_TYPE_CHOICES",
                "A UUID request ID is required.",
            )
        })?
        .to_string();
    let message = input
        .message
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| {
            AppError::invalid(
                "message",
                "BASE_TYPE_REQUIRED",
                "A first message is required.",
            )
        })?;
    if message.len() != 1 || !message.contains_key("content") {
        return Err(AppError::invalid(
            "message",
            "BASE_TYPE_CHOICES",
            "Only text is supported for the first message.",
        ));
    }
    let content = input
        .message
        .as_ref()
        .and_then(|v| v.get("content"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AppError::invalid(
                "message",
                "BASE_TYPE_REQUIRED",
                "A first message is required.",
            )
        })?;
    if content.trim().is_empty() || content.chars().count() > 2000 {
        return Err(AppError::invalid(
            "message",
            "BASE_TYPE_BAD_LENGTH",
            "Message must contain 1-2000 characters.",
        ));
    }
    let digest = hex::encode(Sha256::digest(json!({"name": input.name, "message": content, "tags": input.applied_tags, "duration": input.auto_archive_duration, "slowmode": input.rate_limit_per_user, "invitable": input.invitable}).to_string().as_bytes()));
    let guild_id = parent.guild_id.ok_or_else(AppError::missing_access)?;
    let find = || {
        ports
            .channels
            .find_forum_request(guild_id, parent.id, actor.user_id, &request_key)
    };
    if let Some(existing) = find().await? {
        if existing
            .forum_settings
            .get("request_hash")
            .and_then(Value::as_str)
            != Some(digest.as_str())
        {
            return Err(AppError::Conflict {
                code: 0,
                message: "Request ID was already used with different content.".into(),
            });
        }
        return reconcile_forum_post(ports, actor, existing, content, audit).await;
    }
    super::forum::validate_applied(
        parent,
        &input.applied_tags,
        access.permissions.contains(Permissions::MANAGE_THREADS),
    )?;
    let duration = input.auto_archive_duration.unwrap_or(1440);
    if !AUTO_ARCHIVE_DURATIONS.contains(&duration) {
        return Err(AppError::invalid(
            "auto_archive_duration",
            "BASE_TYPE_CHOICES",
            "Invalid duration.",
        ));
    }
    let now = ports.clock.now();
    // The message service atomically stores the marker and opener in one
    // ten-day partition. Reserve a fresh pair if the clock crosses a boundary.
    let (thread_id, message_id) = loop {
        let thread_id = ports.ids.next_id()?;
        let message_id = ports.ids.next_id()?;
        if thread_id.bucket() == message_id.bucket() {
            break (thread_id, message_id);
        }
    };
    let mut thread = Channel::new(thread_id, ChannelType::PublicThread, parent.guild_id, now);
    apply_name(&mut thread, &input.name)?;
    if let Some(seconds) = input.rate_limit_per_user {
        apply_slowmode(&mut thread, seconds)?;
    }
    thread.parent_id = Some(parent.id);
    thread.owner_id = Some(actor.user_id);
    thread.nsfw = parent.nsfw;
    thread.delivery_mode = parent.delivery_mode;
    thread.applied_tags = input.applied_tags;
    thread.thread_metadata = Some(ThreadMetadata {
        archived: false,
        auto_archive_duration: duration,
        archive_timestamp: now,
        locked: false,
        invitable: input.invitable.unwrap_or(true),
        create_timestamp: now,
    });
    thread.forum_settings = json!({"pending": true, "status": "pending", "request_key": request_key, "request_hash": digest, "message_id": message_id.to_string()});
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::InsertChannel(thread.clone()));
    if let Err(error) = ports.unit_of_work.commit(writes).await {
        if let Some(existing) = find().await? {
            if existing
                .forum_settings
                .get("request_hash")
                .and_then(Value::as_str)
                == Some(digest.as_str())
            {
                return reconcile_forum_post(ports, actor, existing, content, audit).await;
            }
            return Err(AppError::Conflict {
                code: 0,
                message: "Request ID was already used with different content.".into(),
            });
        }
        return Err(error);
    }
    reconcile_forum_post(ports, actor, thread, content, audit).await
}

async fn reconcile_forum_post(
    ports: &Ports,
    actor: &Actor,
    mut thread: Channel,
    content: &str,
    audit: AuditContext<'_>,
) -> AppResult<Channel> {
    let status = thread
        .forum_settings
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("");
    if status == "deleted" {
        return Err(AppError::NotFound(crate::error::Resource::Channel));
    }
    if status == "published" || status == "failed" {
        return Ok(thread);
    }
    let message_id = thread
        .forum_settings
        .get("message_id")
        .and_then(Value::as_str)
        .and_then(|id| id.parse::<Snowflake>().ok())
        .ok_or_else(|| AppError::internal("Invalid forum message reservation"))?;
    let existing = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ports.messages.get(thread.id, message_id),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .flatten();
    let stored = if existing.is_some() {
        existing
    } else {
        let token = ports.ids.next_id()?.to_string();
        if ports
            .channels
            .claim_forum_post(thread.id, "dispatching", &token)
            .await?
        {
            match tokio::time::timeout(std::time::Duration::from_secs(10), ports.messages.create_forum_start(thread.id, json!({"author_id": actor.user_id.to_string(), "message_id": message_id.to_string(), "content": content}))).await {
                Ok(Ok(message)) => Some(message),
                Ok(Err(AppError::NotFound(crate::error::Resource::Message))) => {
                    ports.channels.fail_forum_post(thread.id, &token).await?;
                    None
                },
                _ => None,
            }
        } else {
            None
        }
    };
    if let Some(message) = stored {
        let matches = message.id == message_id
            && message
                .body
                .get("author")
                .and_then(|v| v.get("id"))
                .and_then(Value::as_str)
                == Some(actor.user_id.to_string().as_str())
            && message.body.get("content").and_then(Value::as_str) == Some(content);
        let token = ports.ids.next_id()?.to_string();
        if matches
            && ports
                .channels
                .claim_forum_post(thread.id, "publishing", &token)
                .await?
        {
            thread.forum_settings["pending"] = json!(false);
            thread.forum_settings["status"] = json!("published");
            let guild_id = thread.guild_id.ok_or_else(AppError::missing_access)?;
            let changes = vec![AuditChange::new("name", None, Some(json!(thread.name)))];
            let mut writes = WriteSet::new();
            writes
                .guard_forum_publication(thread.id, token)
                .mutate(Mutation::UpdateChannel(thread.clone()))
                .emit(DomainEvent::ThreadCreate(thread.clone()))
                .emit(DomainEvent::ForumOpeningMessage {
                    guild_id,
                    channel_id: thread.id,
                    message: message.body.clone(),
                })
                .audit(audit.entry(
                    ports,
                    guild_id,
                    AuditAction::ThreadCreate,
                    Some(thread.id),
                    changes,
                )?);
            match ports.unit_of_work.commit(writes).await {
                Ok(()) => return Ok(thread),
                Err(AppError::Conflict { .. }) => {
                    return Ok(ports.channels.find(thread.id).await?.unwrap_or(thread))
                }
                Err(error) => return Err(error),
            }
        }
    }
    Ok(ports.channels.find(thread.id).await?.unwrap_or(thread))
}
