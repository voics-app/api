use serde_json::json;

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
        super::forum::validate_applied(
            parent,
            &input.applied_tags,
            access.permissions.contains(Permissions::MANAGE_THREADS),
        )?;
    } else if !input.applied_tags.is_empty() {
        return Err(AppError::invalid(
            "applied_tags",
            "CHANNEL_TYPE_INVALID",
            "Tags require a forum channel.",
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
