//! `/internal/v1` (P4): calls from sibling services, guarded by `X-Internal-Token`.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;

use application::use_cases::admin::{list_guilds, list_users, stats, suspend_user};
use application::use_cases::internal::{
    channel_access, channel_routing, guild_state, introspect, user_ready, users_lookup,
};
use application::use_cases::push::{forget_token, targets};
use application::{AppError, Resource};
use contracts::internal::{
    AdminGuildDto, AdminUserDto, AdminUserPageDto, ApiAdminStatsDto, ChannelAccessDto,
    ChannelRoutingDto, ForgetPushTokenRequest, IntrospectRequest, IntrospectResponse,
    PushTargetDto, PushTargetsDto, PushTokenDto, SuspendUserRequest, UserReadyDto,
    UsersLookupRequest,
};
use contracts::objects::{
    ChannelDto, GuildFullDto, GuildNotificationSettingsDto, RelationshipDto, UserDto,
    UserSettingsDto,
};
use contracts::wire::{timestamp, Id, Perm};
use domain::push_token::PushPlatform;
use domain::Snowflake;

use crate::error::ApiError;
use crate::extract::{Body, Internal};
use crate::state::AppState;

fn parse_id(raw: &str) -> Result<Snowflake, ApiError> {
    raw.parse()
        .map_err(|_| AppError::NotFound(Resource::User).into())
}

fn region(state: &AppState) -> &str {
    &state.ports.settings.default_voice_region
}

async fn introspect_token(
    State(state): State<AppState>,
    _internal: Internal,
    Body(body): Body<IntrospectRequest>,
) -> Result<Json<IntrospectResponse>, ApiError> {
    let found = introspect::execute(&state.ports, &body.token).await?;
    Ok(Json(IntrospectResponse {
        user_id: Id(found.user_id),
        kind: match found.kind {
            introspect::TokenKind::User => "user",
            introspect::TokenKind::Bot => "bot",
        }
        .to_owned(),
        application_id: found.application_id.map(Id),
        session_id: found.session_id.map(|id| id.to_string()),
    }))
}

async fn users_lookup(
    State(state): State<AppState>,
    _internal: Internal,
    Body(body): Body<UsersLookupRequest>,
) -> Result<Json<Vec<UserDto>>, ApiError> {
    let ids: Vec<Snowflake> = body
        .ids
        .iter()
        .filter_map(|raw| raw.parse().ok())
        .take(users_lookup::LOOKUP_LIMIT)
        .collect();
    let users = users_lookup::execute(&state.ports, &ids).await?;
    Ok(Json(users.iter().map(UserDto::public).collect()))
}

async fn user_ready_state(
    State(state): State<AppState>,
    _internal: Internal,
    Path(user_id): Path<String>,
) -> Result<Json<UserReadyDto>, ApiError> {
    let ready = user_ready::execute(&state.ports, parse_id(&user_id)?).await?;
    let region = region(&state);
    Ok(Json(UserReadyDto {
        user: UserDto::private(&ready.user, &ready.settings.locale),
        guilds: ready
            .guilds
            .iter()
            .map(|snapshot| GuildFullDto::new(snapshot, region))
            .collect(),
        private_channels: ready
            .private_channels
            .iter()
            .map(|view| {
                ChannelDto::private(
                    &view.channel,
                    &view.recipients,
                    view.delivery_mode,
                    Some(ready.user.id),
                )
            })
            .collect(),
        relationships: ready
            .relationships
            .iter()
            .map(|(relationship, user)| RelationshipDto::new(relationship, user))
            .collect(),
        user_settings: UserSettingsDto::from(&ready.settings),
        user_guild_settings: ready
            .guild_settings
            .iter()
            .map(GuildNotificationSettingsDto::from)
            .collect(),
        notes: ready
            .notes
            .iter()
            .map(|(target, note)| (target.to_string(), note.clone()))
            .collect(),
    }))
}

async fn guild_full_state(
    State(state): State<AppState>,
    _internal: Internal,
    Path(guild_id): Path<String>,
) -> Result<Json<GuildFullDto>, ApiError> {
    let snapshot = guild_state::execute(&state.ports, parse_id(&guild_id)?).await?;
    Ok(Json(GuildFullDto::new(&snapshot, region(&state))))
}

async fn channel_delivery(
    State(state): State<AppState>,
    _internal: Internal,
    Path(channel_id): Path<String>,
) -> Result<Json<ChannelRoutingDto>, ApiError> {
    let routing = channel_routing::execute(&state.ports, parse_id(&channel_id)?).await?;
    Ok(Json(ChannelRoutingDto {
        guild_id: routing.guild_id.map(Id),
        channel_type: routing.channel_type.code(),
        delivery_mode: routing.delivery_mode.as_str().to_owned(),
        recipient_ids: routing.recipient_ids.into_iter().map(Id).collect(),
    }))
}

async fn channel_access_info(
    State(state): State<AppState>,
    _internal: Internal,
    Path((channel_id, user_id)): Path<(String, String)>,
) -> Result<Json<ChannelAccessDto>, ApiError> {
    let info =
        channel_access::execute(&state.ports, parse_id(&channel_id)?, parse_id(&user_id)?).await?;
    Ok(Json(ChannelAccessDto {
        permissions: Perm(info.permissions),
        guild_id: info.guild_id.map(Id),
        channel_type: info.channel_type.code(),
        delivery_mode: info.delivery_mode.as_str().to_owned(),
        recipient_ids: info.recipient_ids.into_iter().map(Id).collect(),
    }))
}

#[derive(Deserialize)]
struct PushTargetsQuery {
    author_id: String,
}

async fn push_targets(
    State(state): State<AppState>,
    _internal: Internal,
    Path(channel_id): Path<String>,
    Query(params): Query<PushTargetsQuery>,
) -> Result<Json<PushTargetsDto>, ApiError> {
    let found = targets::execute(
        &state.ports,
        parse_id(&channel_id)?,
        parse_id(&params.author_id)?,
    )
    .await?;
    Ok(Json(PushTargetsDto {
        guild_id: found.guild_id.map(Id),
        guild_name: found.guild_name,
        channel_name: found.channel_name,
        delivery_mode: found.delivery_mode.as_str().to_owned(),
        targets: found
            .targets
            .into_iter()
            .map(|target| PushTargetDto {
                user_id: Id(target.user_id),
                role_ids: target.role_ids.into_iter().map(Id).collect(),
                online: target.online,
                level: target.level.code(),
                muted: target.muted,
                suppress_everyone: target.suppress_everyone,
                suppress_roles: target.suppress_roles,
                tokens: target
                    .tokens
                    .into_iter()
                    .map(|token| PushTokenDto {
                        platform: token.platform.as_str().to_owned(),
                        token: token.token,
                        environment: token.environment.as_str().to_owned(),
                        previews: token.previews,
                        only_mentions: token.only_mentions,
                    })
                    .collect(),
            })
            .collect(),
    }))
}

async fn forget_push_token(
    State(state): State<AppState>,
    _internal: Internal,
    Body(body): Body<ForgetPushTokenRequest>,
) -> Result<StatusCode, ApiError> {
    let platform = PushPlatform::parse(&body.platform).ok_or_else(|| {
        AppError::invalid("platform", "PUSH_PLATFORM_INVALID", "Must be apns or fcm.")
    })?;
    forget_token::execute(&state.ports, platform, &body.token).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn admin_stats(
    State(state): State<AppState>,
    _internal: Internal,
) -> Result<Json<ApiAdminStatsDto>, ApiError> {
    let figures = stats::execute(&state.ports).await?;
    Ok(Json(ApiAdminStatsDto {
        users_total: figures.users_total,
        guilds_total: figures.guilds_total,
        channels_total: figures.channels_total,
        bots_total: figures.bots_total,
        signups_24h: figures.signups_24h,
    }))
}

#[derive(Deserialize)]
struct UserQuery {
    query: Option<String>,
    limit: Option<u32>,
    after: Option<String>,
}

async fn admin_users(
    State(state): State<AppState>,
    _internal: Internal,
    Query(params): Query<UserQuery>,
) -> Result<Json<AdminUserPageDto>, ApiError> {
    let after = params.after.as_deref().map(parse_id).transpose()?;
    let page =
        list_users::execute(&state.ports, params.query.as_deref(), params.limit, after).await?;
    Ok(Json(AdminUserPageDto {
        users: page
            .users
            .iter()
            .map(|user| AdminUserDto {
                id: Id(user.id),
                username: user.username.clone(),
                email: user.email.clone(),
                bot: user.bot,
                created_at: timestamp(user.created_at),
                suspended_until: user
                    .suspension
                    .as_ref()
                    .and_then(|s| s.until)
                    .map(timestamp),
                suspension_reason: user.suspension.as_ref().map(|s| s.reason.clone()),
            })
            .collect(),
        next_after: page.next_after.map(Id),
    }))
}

async fn admin_suspend(
    State(state): State<AppState>,
    _internal: Internal,
    Path(user_id): Path<String>,
    Body(body): Body<SuspendUserRequest>,
) -> Result<StatusCode, ApiError> {
    suspend_user::execute(&state.ports, parse_id(&user_id)?, body.reason, body.until).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct GuildQuery {
    query: Option<String>,
}

async fn admin_guilds(
    State(state): State<AppState>,
    _internal: Internal,
    Query(params): Query<GuildQuery>,
) -> Result<Json<Vec<AdminGuildDto>>, ApiError> {
    let rows = list_guilds::execute(&state.ports, params.query.as_deref()).await?;
    Ok(Json(
        rows.into_iter()
            .map(|(guild, member_count)| AdminGuildDto {
                id: Id(guild.id),
                name: guild.name,
                owner_id: Id(guild.owner_id),
                member_count,
                voice_region: guild
                    .voice_region
                    .unwrap_or_else(|| region(&state).to_owned()),
                created_at: timestamp(guild.created_at),
            })
            .collect(),
    ))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/internal/v1/auth/introspect", post(introspect_token))
        .route("/internal/v1/users/lookup", post(users_lookup))
        .route("/internal/v1/users/{id}/ready", get(user_ready_state))
        .route("/internal/v1/guilds/{id}/state", get(guild_full_state))
        .route(
            "/internal/v1/channels/{id}/access/{user_id}",
            get(channel_access_info),
        )
        .route("/internal/v1/channels/{id}/delivery", get(channel_delivery))
        .route("/internal/v1/channels/{id}/push-targets", get(push_targets))
        .route("/internal/v1/push-tokens", delete(forget_push_token))
        .route("/internal/v1/admin/stats", get(admin_stats))
        .route("/internal/v1/admin/users", get(admin_users))
        .route("/internal/v1/admin/users/{id}/suspend", post(admin_suspend))
        .route("/internal/v1/admin/guilds", get(admin_guilds))
}
