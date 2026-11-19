//! Channels, threads, permission overwrites and typing (P9).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};

use application::events::ChannelView;
use application::support::audit::AuditContext;
use application::use_cases::channels::create_guild_channel::CreateChannelInput;
use application::use_cases::channels::create_thread::CreateThreadInput;
use application::use_cases::channels::reorder_channels::ChannelPosition;
use application::use_cases::channels::update_channel::UpdateChannelInput;
use application::use_cases::channels::{
    add_recipient, create_guild_channel, create_thread, delete_channel, delete_overwrite,
    get_channel, list_guild_channels, put_overwrite, remove_recipient, reorder_channels,
    trigger_typing, update_channel,
};
use application::{AppError, Resource};
use contracts::objects::ChannelDto;
use contracts::requests::{
    ChannelPositionRequest, CreateChannelRequest, CreateThreadRequest, EditOverwriteRequest,
    ModifyChannelRequest, OverwriteRequest,
};
use contracts::wire::{Id, Perm};
use domain::channel::ChannelType;
use domain::delivery_mode::DeliveryMode;
use domain::permissions::{OverwriteKind, PermissionOverwrite, Permissions};
use domain::Snowflake;

use crate::error::ApiError;
use crate::extract::{AuditReason, Body, Caller};
use crate::images;
use crate::routes::ids::snowflake;
use crate::state::AppState;

fn delivery(raw: Option<String>) -> Result<Option<DeliveryMode>, AppError> {
    raw.as_deref()
        .map(|value| {
            value.parse::<DeliveryMode>().map_err(|_| {
                AppError::invalid(
                    "delivery_mode",
                    "INVALID_DELIVERY_MODE",
                    "Unknown delivery mode.",
                )
            })
        })
        .transpose()
}

fn kind_code(raw: u8, field: &str) -> Result<ChannelType, AppError> {
    ChannelType::from_code(raw)
        .ok_or_else(|| AppError::invalid(field, "INVALID_CHANNEL_TYPE", "Unknown channel type."))
}

fn overwrite(request: OverwriteRequest) -> Result<PermissionOverwrite, AppError> {
    Ok(PermissionOverwrite {
        id: request.id.0,
        kind: overwrite_kind(request.kind)?,
        allow: request
            .allow
            .map(|Perm(p)| p)
            .unwrap_or(Permissions::from_bits(0)),
        deny: request
            .deny
            .map(|Perm(p)| p)
            .unwrap_or(Permissions::from_bits(0)),
    })
}

fn overwrite_kind(code: u8) -> Result<OverwriteKind, AppError> {
    OverwriteKind::from_code(code).ok_or_else(|| {
        AppError::invalid(
            "type",
            "INVALID_OVERWRITE_TYPE",
            "Overwrite type must be 0 or 1.",
        )
    })
}

fn view_dto(view: &ChannelView, viewer: Snowflake) -> ChannelDto {
    match view {
        ChannelView::Guild(channel) => ChannelDto::guild(channel),
        ChannelView::Private(private) => ChannelDto::private(
            &private.channel,
            &private.recipients,
            private.delivery_mode,
            Some(viewer),
        ),
    }
}

async fn list(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(guild): Path<String>,
) -> Result<Json<Vec<ChannelDto>>, ApiError> {
    let channels =
        list_guild_channels::execute(&state.ports, &actor, snowflake(&guild, Resource::Guild)?)
            .await?;
    Ok(Json(channels.iter().map(ChannelDto::guild).collect()))
}

async fn create(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path(guild): Path<String>,
    Body(body): Body<CreateChannelRequest>,
) -> Result<(StatusCode, Json<ChannelDto>), ApiError> {
    let kind = body.kind.map(|code| kind_code(code, "type")).transpose()?;
    let input = CreateChannelInput {
        name: body.name,
        kind: kind.map(|k| k.code()),
        topic: body.topic,
        bitrate: body.bitrate,
        user_limit: body.user_limit,
        rate_limit_per_user: body.rate_limit_per_user,
        position: body.position,
        parent_id: body.parent_id.map(|Id(id)| id),
        nsfw: body.nsfw,
        permission_overwrites: body
            .permission_overwrites
            .into_iter()
            .map(overwrite)
            .collect::<Result<_, _>>()?,
        delivery_mode: delivery(body.delivery_mode)?,
        voice_region: body.voice_region,
    };
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    let channel = create_guild_channel::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        input,
        audit,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(ChannelDto::guild(&channel))))
}

async fn reorder(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(guild): Path<String>,
    Body(body): Body<Vec<ChannelPositionRequest>>,
) -> Result<StatusCode, ApiError> {
    let moves = body
        .into_iter()
        .map(|item| ChannelPosition {
            id: item.id.0,
            position: item.position.flatten(),
            parent_id: item.parent_id.map(|inner| inner.map(|Id(id)| id)),
            lock_permissions: item.lock_permissions.flatten().unwrap_or(false),
        })
        .collect();
    reorder_channels::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        moves,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn fetch(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
) -> Result<Json<ChannelDto>, ApiError> {
    let view =
        get_channel::execute(&state.ports, &actor, snowflake(&id, Resource::Channel)?).await?;
    Ok(Json(view_dto(&view, actor.user_id)))
}

async fn modify(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path(id): Path<String>,
    Body(body): Body<ModifyChannelRequest>,
) -> Result<Json<ChannelDto>, ApiError> {
    let input = UpdateChannelInput {
        name: body.name,
        topic: body.topic,
        icon: images::patch(body.icon)?,
        bitrate: body.bitrate,
        user_limit: body.user_limit,
        rate_limit_per_user: body.rate_limit_per_user,
        position: body.position,
        parent_id: body.parent_id.map(|inner| inner.map(|Id(id)| id)),
        nsfw: body.nsfw,
        text_chat_enabled: body.text_chat_enabled,
        permission_overwrites: body
            .permission_overwrites
            .map(|items| {
                items
                    .into_iter()
                    .map(overwrite)
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?,
        delivery_mode: delivery(body.delivery_mode)?,
        voice_region: body.voice_region,
        archived: body.archived,
        locked: body.locked,
        auto_archive_duration: body.auto_archive_duration,
        available_tags: body.available_tags,
        default_sort_order: body.default_sort_order,
        default_forum_layout: body.default_forum_layout,
        default_reaction_emoji: body.default_reaction_emoji,
        require_tags: body.require_tags,
        applied_tags: body.applied_tags,
    };
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    let view = update_channel::execute(
        &state.ports,
        &actor,
        snowflake(&id, Resource::Channel)?,
        input,
        audit,
    )
    .await?;
    Ok(Json(view_dto(&view, actor.user_id)))
}

async fn remove(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path(id): Path<String>,
) -> Result<Json<ChannelDto>, ApiError> {
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    let view = delete_channel::execute(
        &state.ports,
        &actor,
        snowflake(&id, Resource::Channel)?,
        audit,
    )
    .await?;
    Ok(Json(view_dto(&view, actor.user_id)))
}

async fn add_group_recipient(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path((channel, user)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    add_recipient::execute(
        &state.ports,
        &actor,
        snowflake(&channel, Resource::Channel)?,
        snowflake(&user, Resource::User)?,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_group_recipient(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path((channel, user)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let target = if user == "@me" {
        None
    } else {
        Some(snowflake(&user, Resource::User)?)
    };
    remove_recipient::execute(
        &state.ports,
        &actor,
        snowflake(&channel, Resource::Channel)?,
        target,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn put_permission(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path((channel, target)): Path<(String, String)>,
    Body(body): Body<EditOverwriteRequest>,
) -> Result<StatusCode, ApiError> {
    let value = PermissionOverwrite {
        id: snowflake(&target, Resource::Overwrite)?,
        kind: overwrite_kind(body.kind)?,
        allow: body
            .allow
            .map(|Perm(p)| p)
            .unwrap_or(Permissions::from_bits(0)),
        deny: body
            .deny
            .map(|Perm(p)| p)
            .unwrap_or(Permissions::from_bits(0)),
    };
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    put_overwrite::execute(
        &state.ports,
        &actor,
        snowflake(&channel, Resource::Channel)?,
        value,
        audit,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_permission(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path((channel, target)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    delete_overwrite::execute(
        &state.ports,
        &actor,
        snowflake(&channel, Resource::Channel)?,
        snowflake(&target, Resource::Overwrite)?,
        audit,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn thread(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path(parent): Path<String>,
    Body(body): Body<CreateThreadRequest>,
) -> Result<(StatusCode, Json<ChannelDto>), ApiError> {
    if body.message.is_some() {
        return Err(application::AppError::invalid(
            "message",
            "NOT_YET_SUPPORTED",
            "Creating the first message with a thread is not supported; create it separately.",
        )
        .into());
    }
    let kind = body.kind.map(|code| kind_code(code, "type")).transpose()?;
    let input = CreateThreadInput {
        name: body.name,
        kind: kind.map(|k| k.code()),
        auto_archive_duration: body.auto_archive_duration,
        invitable: body.invitable,
        rate_limit_per_user: body.rate_limit_per_user,
        applied_tags: body.applied_tags,
    };
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    let channel = create_thread::execute(
        &state.ports,
        &actor,
        snowflake(&parent, Resource::Channel)?,
        input,
        audit,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(ChannelDto::guild(&channel))))
}

async fn typing(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    trigger_typing::execute(&state.ports, &actor, snowflake(&id, Resource::Channel)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/guilds/{id}/channels",
            get(list).post(create).patch(reorder),
        )
        .route("/channels/{id}", get(fetch).patch(modify).delete(remove))
        .route(
            "/channels/{id}/permissions/{overwrite_id}",
            put(put_permission).delete(remove_permission),
        )
        .route(
            "/channels/{id}/recipients/{user_id}",
            put(add_group_recipient).delete(remove_group_recipient),
        )
        .route("/channels/{id}/threads", post(thread))
        .route("/channels/{id}/typing", post(typing))
}
