//! `/internal/v1` (P4): calls from sibling services, guarded by `X-Internal-Token`.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use application::use_cases::admin::{
    list_guilds, list_reports, list_users, resolve_report, stats, suspend_user,
};
use application::use_cases::internal::{
    channel_access, channel_routing, guild_state, introspect, user_ready, users_lookup,
};
use application::use_cases::push::{
    forget_token, manual_targets, notification_recipients, targets,
};
use application::{AppError, Resource};
use contracts::internal::{
    AdminGuildDto, AdminReportDto, AdminReportPageDto, AdminReportUserDto, AdminUserDto,
    AdminUserPageDto, ApiAdminStatsDto, ChannelAccessDto, ChannelRoutingDto,
    ForgetPushTokenRequest, InternalGuildFullDto, IntrospectRequest, IntrospectResponse,
    ManualPushTargetsDto, ManualPushUserDto, NotificationRecipientDto,
    NotificationRecipientPageDto, PushTargetDto, PushTargetsDto, PushTokenDto,
    ResolveReportRequest, SuspendUserRequest, UserReadyDto, UsersLookupRequest, VoipTokenDto,
};
use contracts::objects::{
    ChannelDto, GuildNotificationSettingsDto, RelationshipDto, UserDto, UserSettingsDto,
};
use contracts::wire::{timestamp, Id, Perm};
use domain::push_token::PushPlatform;
use domain::report::{Report, ReportStatus, Resolution};
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

async fn reserve_ids(
    State(state): State<AppState>,
    _internal: Internal,
    Body(body): Body<voics_snowflake::ReservationRequest>,
) -> Result<Json<voics_snowflake::Reservation>, ApiError> {
    if body.service() != voics_snowflake::Service::Messages {
        return Err(AppError::invalid(
            "service",
            "INVALID_SERVICE",
            "Only messages may reserve through HTTP",
        )
        .into());
    }
    Ok(Json(state.ports.id_reservations.reserve(body).await?))
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
            .map(|snapshot| InternalGuildFullDto::new(snapshot, region))
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
) -> Result<Json<InternalGuildFullDto>, ApiError> {
    let snapshot = guild_state::execute(&state.ports, parse_id(&guild_id)?).await?;
    Ok(Json(InternalGuildFullDto::new(&snapshot, region(&state))))
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
                voip_tokens: target
                    .voip_tokens
                    .into_iter()
                    .map(|token| VoipTokenDto {
                        token: token.token,
                        environment: token.environment.as_str().to_owned(),
                    })
                    .collect(),
            })
            .collect(),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NotificationRecipientsQuery {
    author_id: String,
    after: Option<String>,
    limit: Option<u32>,
}

fn notification_id(field: &str, raw: &str) -> Result<Snowflake, ApiError> {
    match raw.parse::<i64>() {
        Ok(id) if id >= 0 && raw.bytes().all(|byte| byte.is_ascii_digit()) => {
            Ok(Snowflake::from_i64(id))
        }
        _ => Err(AppError::invalid(
            field,
            "BASE_TYPE_BAD_FORMAT",
            "Must be a decimal ID in the database's signed 64-bit range.",
        )
        .into()),
    }
}

async fn notification_recipient_page(
    State(state): State<AppState>,
    _internal: Internal,
    Path(channel_id): Path<String>,
    Query(params): Query<NotificationRecipientsQuery>,
) -> Result<Json<NotificationRecipientPageDto>, ApiError> {
    let page = notification_recipients::execute(
        &state.ports,
        notification_id("channel_id", &channel_id)?,
        notification_id("author_id", &params.author_id)?,
        params
            .after
            .as_deref()
            .map(|id| notification_id("after", id))
            .transpose()?,
        params
            .limit
            .unwrap_or(notification_recipients::DEFAULT_LIMIT),
    )
    .await?;
    Ok(Json(NotificationRecipientPageDto {
        channel_id: Id(page.channel_id),
        guild_id: page.guild_id.map(Id),
        delivery_mode: page.delivery_mode.as_str().to_owned(),
        evaluated_at: timestamp(page.evaluated_at),
        next_after: page.next_after.map(Id),
        recipients: page
            .recipients
            .into_iter()
            .map(|recipient| NotificationRecipientDto {
                user_id: Id(recipient.user_id),
                role_ids: recipient.role_ids.into_iter().map(Id).collect(),
                online: recipient.online,
                level: recipient.level.code(),
                muted: recipient.muted,
                suppress_everyone: recipient.suppress_everyone,
                suppress_roles: recipient.suppress_roles,
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

fn push_token_dto(token: domain::push_token::PushToken) -> PushTokenDto {
    PushTokenDto {
        platform: token.platform.as_str().to_owned(),
        token: token.token,
        environment: token.environment.as_str().to_owned(),
        previews: token.previews,
        only_mentions: token.only_mentions,
    }
}

fn manual_targets_dto(found: manual_targets::ManualTargets) -> ManualPushTargetsDto {
    ManualPushTargetsDto {
        users: found
            .users
            .into_iter()
            .map(|(user_id, tokens)| ManualPushUserDto {
                user_id: Id(user_id),
                tokens: tokens.into_iter().map(push_token_dto).collect(),
            })
            .collect(),
        next_after: found.next_after.map(Id),
    }
}

async fn user_push_tokens(
    State(state): State<AppState>,
    _internal: Internal,
    Path(user_id): Path<String>,
) -> Result<Json<ManualPushTargetsDto>, ApiError> {
    let found = manual_targets::for_user(&state.ports, parse_id(&user_id)?).await?;
    Ok(Json(manual_targets_dto(found)))
}

#[derive(Deserialize)]
struct EventReminderTargetsQuery {
    user_id: String,
    scheduled_start_time: chrono::DateTime<chrono::Utc>,
    revision: i64,
}

async fn event_reminder_targets(
    State(state): State<AppState>,
    _internal: Internal,
    Path((guild_id, event_id)): Path<(String, String)>,
    Query(query): Query<EventReminderTargetsQuery>,
) -> Result<Json<ManualPushTargetsDto>, ApiError> {
    let user_id = parse_id(&query.user_id)?;
    let tokens = application::use_cases::scheduled_events::reminder_targets(
        &state.ports,
        parse_id(&guild_id)?,
        parse_id(&event_id)?,
        user_id,
        query.scheduled_start_time,
        query.revision,
    )
    .await?;
    let users = if tokens.is_empty() {
        Vec::new()
    } else {
        vec![(user_id, tokens)]
    };
    Ok(Json(manual_targets_dto(manual_targets::ManualTargets {
        users,
        next_after: None,
    })))
}

#[derive(Deserialize)]
struct DeliveryLeaseRequest {
    lease_id: uuid::Uuid,
}

#[derive(Deserialize)]
struct FinishDeliveryRequest {
    lease_id: uuid::Uuid,
    retry_after_secs: Option<u32>,
}

async fn claim_event_delivery(
    State(state): State<AppState>,
    _internal: Internal,
) -> Result<Json<Value>, ApiError> {
    let delivery = state.ports.guilds.claim_scheduled_event_delivery().await?;
    Ok(Json(match delivery {
        None => Value::Null,
        Some(delivery) => json!({
            "delivery_id": delivery.id, "lease_id": delivery.lease_id,
            "guild_id": delivery.guild_id.to_string(), "event_id": delivery.event_id.to_string(),
            "user_id": delivery.user_id.to_string(), "scheduled_start_time": timestamp(delivery.start),
            "revision": delivery.schedule_revision
        }),
    }))
}

async fn resolve_event_delivery(
    State(state): State<AppState>,
    _internal: Internal,
    Path(id): Path<uuid::Uuid>,
    Body(body): Body<DeliveryLeaseRequest>,
) -> Result<Json<Value>, ApiError> {
    let delivery = state
        .ports
        .guilds
        .scheduled_event_delivery(id, body.lease_id)
        .await?
        .ok_or(AppError::Conflict {
            code: 0,
            message: "Reminder lease expired.".into(),
        })?;
    let tokens = application::use_cases::scheduled_events::reminder_targets(
        &state.ports,
        delivery.guild_id,
        delivery.event_id,
        delivery.user_id,
        delivery.start,
        delivery.schedule_revision,
    )
    .await?;
    let token = tokens
        .into_iter()
        .find(|token| token.session_id == delivery.session_id);
    Ok(Json(match token {
        Some(token) => serde_json::to_value(push_token_dto(token)).map_err(AppError::internal)?,
        None => Value::Null,
    }))
}

async fn finish_event_delivery(
    State(state): State<AppState>,
    _internal: Internal,
    Path(id): Path<uuid::Uuid>,
    Body(body): Body<FinishDeliveryRequest>,
) -> Result<StatusCode, ApiError> {
    if !state
        .ports
        .guilds
        .finish_scheduled_event_delivery(id, body.lease_id, body.retry_after_secs)
        .await?
    {
        return Err(AppError::Conflict {
            code: 0,
            message: "Reminder lease expired.".into(),
        }
        .into());
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct PushTokenPageQuery {
    after: Option<String>,
    limit: Option<u32>,
}

async fn push_token_page(
    State(state): State<AppState>,
    _internal: Internal,
    Query(params): Query<PushTokenPageQuery>,
) -> Result<Json<ManualPushTargetsDto>, ApiError> {
    let after = params.after.as_deref().map(parse_id).transpose()?;
    let found = manual_targets::page(&state.ports, after, params.limit).await?;
    Ok(Json(manual_targets_dto(found)))
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

#[derive(Deserialize)]
struct ReportQuery {
    status: Option<String>,
    limit: Option<u32>,
    after: Option<String>,
}

fn report_dto(
    report: &Report,
    reporter: Option<&domain::user::User>,
    reported: Option<&domain::user::User>,
) -> AdminReportDto {
    let who = |id: Snowflake, user: Option<&domain::user::User>| AdminReportUserDto {
        id: Id(id),
        username: user.map(|u| u.username.clone()),
    };
    AdminReportDto {
        id: Id(report.id),
        kind: report.kind.as_str().to_owned(),
        category: report.category.as_str().to_owned(),
        note: report.note.clone(),
        status: report.status.as_str().to_owned(),
        created_at: timestamp(report.created_at),
        reporter: who(report.reporter_id, reporter),
        reported: who(report.reported_user_id, reported),
        channel_id: report.channel_id.map(Id),
        guild_id: report.guild_id.map(Id),
        message_id: report.message_id.map(Id),
        delivery_mode: report.delivery_mode.map(|m| m.as_str().to_owned()),
        content: report.content.clone(),
        content_source: report.content_source.map(|s| s.as_str().to_owned()),
        resolved_at: report.resolved_at.map(timestamp),
        resolved_by: report.resolved_by.clone(),
        resolution_note: report.resolution_note.clone(),
    }
}

async fn admin_reports(
    State(state): State<AppState>,
    _internal: Internal,
    Query(params): Query<ReportQuery>,
) -> Result<Json<AdminReportPageDto>, ApiError> {
    let status = match params.status.as_deref() {
        None => ReportStatus::Open,
        Some(raw) => ReportStatus::parse(raw).ok_or_else(|| {
            AppError::invalid(
                "status",
                "REPORT_STATUS_INVALID",
                "Must be open, resolved or dismissed.",
            )
        })?,
    };
    let after = params.after.as_deref().map(parse_id).transpose()?;
    let page = list_reports::execute(&state.ports, status, params.limit, after).await?;
    Ok(Json(AdminReportPageDto {
        reports: page
            .reports
            .iter()
            .map(|view| report_dto(&view.report, view.reporter.as_ref(), view.reported.as_ref()))
            .collect(),
        next_after: page.next_after.map(Id),
    }))
}

async fn admin_resolve_report(
    State(state): State<AppState>,
    _internal: Internal,
    Path(report_id): Path<String>,
    Body(body): Body<ResolveReportRequest>,
) -> Result<Json<AdminReportDto>, ApiError> {
    let id = report_id
        .parse()
        .map_err(|_| AppError::NotFound(Resource::Report))?;
    let resolution = Resolution::parse(&body.resolution).map_err(AppError::from)?;
    let report = resolve_report::execute(
        &state.ports,
        id,
        resolution,
        &body.resolved_by,
        body.note.as_deref(),
    )
    .await?;
    Ok(Json(report_dto(&report, None, None)))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/internal/v1/id-reservations", post(reserve_ids))
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
        .route(
            "/internal/v1/channels/{id}/notification-recipients",
            get(notification_recipient_page),
        )
        .route(
            "/internal/v1/push-tokens",
            delete(forget_push_token).get(push_token_page),
        )
        .route("/internal/v1/users/{id}/push-tokens", get(user_push_tokens))
        .route(
            "/internal/v1/guilds/{guild_id}/scheduled-events/{event_id}/push-targets",
            get(event_reminder_targets),
        )
        .route(
            "/internal/v1/scheduled-event-deliveries/claim",
            post(claim_event_delivery),
        )
        .route(
            "/internal/v1/scheduled-event-deliveries/{id}/resolve",
            post(resolve_event_delivery),
        )
        .route(
            "/internal/v1/scheduled-event-deliveries/{id}/finish",
            post(finish_event_delivery),
        )
        .route("/internal/v1/admin/stats", get(admin_stats))
        .route("/internal/v1/admin/users", get(admin_users))
        .route("/internal/v1/admin/users/{id}/suspend", post(admin_suspend))
        .route("/internal/v1/admin/guilds", get(admin_guilds))
        .route("/internal/v1/admin/reports", get(admin_reports))
        .route(
            "/internal/v1/admin/reports/{id}/resolve",
            post(admin_resolve_report),
        )
}
