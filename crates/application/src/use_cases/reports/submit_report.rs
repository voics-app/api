use serde_json::Value;

use domain::delivery_mode::DeliveryMode;
use domain::report::{
    clean_note, clean_submitted_content, ContentSource, Report, ReportCategory, ReportKind,
    ReportStatus,
};
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::support::channel_access::ChannelAccess;
use crate::support::delivery::{effective_mode, viewer_ids};
use crate::support::throttle;
use crate::support::views::require_user;

/// Reports a user may file per [`REPORT_WINDOW`].
pub const REPORTS_PER_WINDOW: u32 = 10;
pub const REPORT_WINDOW: std::time::Duration = std::time::Duration::from_secs(60 * 60);

#[derive(Debug, Clone)]
pub enum ReportTarget {
    Guild {
        guild_id: Snowflake,
    },
    Message {
        channel_id: Snowflake,
        message_id: Snowflake,
        /// Ephemeral and P2P chats only: who wrote the message, as the reporter saw it.
        author_id: Option<Snowflake>,
        /// Ephemeral and P2P chats only: the text the reporter submits as evidence.
        content: Option<String>,
    },
    User {
        user_id: Snowflake,
    },
}

#[derive(Debug, Clone)]
pub struct SubmitReportInput {
    pub target: ReportTarget,
    pub category: String,
    pub note: Option<String>,
}

/// The report as far as the target resolves: who is reported and what evidence the server holds.
struct Resolved {
    reported_user_id: Snowflake,
    kind: ReportKind,
    channel_id: Option<Snowflake>,
    guild_id: Option<Snowflake>,
    message_id: Option<Snowflake>,
    delivery_mode: Option<DeliveryMode>,
    content: Option<String>,
    content_source: Option<ContentSource>,
}

/// `POST /reports`: files a report of a message or a user for the Trust and Safety queue.
/// Repeating an open report is a silent no-op, so the reporter learns nothing about earlier ones.
pub async fn execute(ports: &Ports, actor: &Actor, input: SubmitReportInput) -> AppResult<()> {
    actor.require_user()?;
    let category = ReportCategory::parse(&input.category)?;
    let note = clean_note(input.note.as_deref())?;
    throttle::hit(
        ports,
        &format!("report:{}", actor.user_id),
        REPORTS_PER_WINDOW,
        REPORT_WINDOW,
    )
    .await?;
    let resolved = match input.target {
        ReportTarget::Guild { guild_id } => guild_target(ports, guild_id).await?,
        ReportTarget::User { user_id } => user_target(ports, user_id).await?,
        ReportTarget::Message {
            channel_id,
            message_id,
            author_id,
            content,
        } => message_target(ports, actor, channel_id, message_id, author_id, content).await?,
    };
    if resolved.reported_user_id == actor.user_id {
        return Err(AppError::invalid(
            "target",
            "REPORT_SELF",
            "You cannot report yourself.",
        ));
    }
    let report = Report {
        id: ports.ids.next_id()?,
        reporter_id: actor.user_id,
        reported_user_id: resolved.reported_user_id,
        kind: resolved.kind,
        category,
        note,
        channel_id: resolved.channel_id,
        guild_id: resolved.guild_id,
        message_id: resolved.message_id,
        delivery_mode: resolved.delivery_mode,
        content: resolved.content,
        content_source: resolved.content_source,
        status: ReportStatus::Open,
        created_at: ports.clock.now(),
        resolved_at: None,
        resolved_by: None,
        resolution_note: None,
    };
    ports.reports.insert(&report).await?;
    Ok(())
}

async fn user_target(ports: &Ports, user_id: Snowflake) -> AppResult<Resolved> {
    require_user(ports, user_id).await?;
    Ok(Resolved {
        reported_user_id: user_id,
        kind: ReportKind::User,
        channel_id: None,
        guild_id: None,
        message_id: None,
        delivery_mode: None,
        content: None,
        content_source: None,
    })
}

async fn guild_target(ports: &Ports, guild_id: Snowflake) -> AppResult<Resolved> {
    let guild = ports
        .guilds
        .find(guild_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Guild))?;
    let listing = ports
        .guilds
        .discovery(guild_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Guild))?;
    if !domain::discovery::ineligibilities(
        &guild,
        ports.guilds.member_count(guild_id).await?,
        ports.guilds.has_nsfw_channel(guild_id).await?,
        ports.settings.discovery_rules(),
        ports.clock.now(),
    )
    .is_empty()
    {
        return Err(AppError::NotFound(Resource::Guild));
    }
    Ok(Resolved {
        reported_user_id: guild.owner_id,
        kind: ReportKind::Guild,
        guild_id: Some(guild_id),
        channel_id: None,
        message_id: None,
        delivery_mode: None,
        content: Some(format!(
            "Name: {}\nDescription: {}\nCategory: {}\nTags: {}",
            guild.name,
            guild.description.unwrap_or_default(),
            listing.category,
            listing.tags.join(", ")
        )),
        content_source: Some(ContentSource::Server),
    })
}

async fn message_target(
    ports: &Ports,
    actor: &Actor,
    channel_id: Snowflake,
    message_id: Snowflake,
    author_id: Option<Snowflake>,
    submitted: Option<String>,
) -> AppResult<Resolved> {
    let access = ChannelAccess::visible(ports, channel_id, actor.user_id).await?;
    let mode = effective_mode(ports, &access.channel).await?;
    let (reported_user_id, content, content_source) = if mode == DeliveryMode::Standard {
        let stored = ports
            .messages
            .get(channel_id, message_id)
            .await?
            .ok_or(AppError::NotFound(Resource::Message))?;
        let author = stored
            .body
            .pointer("/author/id")
            .and_then(Value::as_str)
            .and_then(|raw| raw.parse::<Snowflake>().ok())
            .ok_or_else(|| AppError::Unavailable("messages returned no author".into()))?;
        let content = stored
            .body
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_owned);
        (author, content, Some(ContentSource::Server))
    } else {
        let author = author_id.ok_or_else(|| {
            AppError::invalid(
                "author_id",
                "BASE_TYPE_REQUIRED",
                "Required for ephemeral and P2P chats, which the server does not store.",
            )
        })?;
        let participants = match &access.guild {
            Some(guild) => {
                viewer_ids(
                    ports,
                    &guild.guild,
                    &guild.roles,
                    &access.channel,
                    access.parent.as_ref(),
                )
                .await?
            }
            None => access.channel.recipient_ids.clone(),
        };
        if !participants.contains(&author) {
            return Err(AppError::invalid(
                "author_id",
                "REPORT_AUTHOR_NOT_IN_CHANNEL",
                "The author is not in this channel.",
            ));
        }
        let content = clean_submitted_content(submitted.as_deref())?;
        let source = content.as_ref().map(|_| ContentSource::Reporter);
        (author, content, source)
    };
    require_user(ports, reported_user_id).await?;
    Ok(Resolved {
        reported_user_id,
        kind: ReportKind::Message,
        channel_id: Some(channel_id),
        guild_id: access.channel.guild_id,
        message_id: Some(message_id),
        delivery_mode: Some(mode),
        content,
        content_source,
    })
}
