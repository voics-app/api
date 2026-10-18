//! `/api/v1/users` (P9): profile, settings, private channels, delivery modes
//! and relationships.

use axum::body::Bytes;
use axum::extract::DefaultBodyLimit;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};

use application::use_cases::relationships::{
    list_relationships, put_relationship, remove_relationship, send_friend_request,
};
use application::use_cases::users::update_current_user::UpdateCurrentUserInput;
use application::use_cases::users::update_settings::UpdateSettingsInput;
use application::use_cases::users::{
    get_current_user, get_settings, get_user, list_private_channels, open_private_channel,
    set_delivery_override, update_current_user, update_settings,
};
use application::AppError;
use contracts::objects::{ChannelDto, RelationshipDto, UserDto, UserSettingsDto};
use contracts::requests::{
    CreateDmRequest, ModifyCurrentUserRequest, ModifyUserSettingsRequest, PutRelationshipRequest,
    SetDeliveryModeRequest,
};
use contracts::wire::Id;
use domain::delivery_mode::DeliveryMode;
use domain::user::CustomStatus;
use domain::Snowflake;

use crate::error::ApiError;
use crate::extract::{Body, Caller};
use crate::images::{self, IMAGE_BODY_LIMIT};
use crate::state::AppState;

fn parse_id(raw: &str) -> Result<Snowflake, ApiError> {
    raw.parse()
        .map_err(|_| AppError::NotFound(application::Resource::User).into())
}

async fn current_user(
    State(state): State<AppState>,
    Caller(actor): Caller,
) -> Result<Json<UserDto>, ApiError> {
    let user = get_current_user::execute(&state.ports, &actor).await?;
    let locale = state.ports.users.settings(user.id).await?.locale;
    Ok(Json(UserDto::private(&user, &locale)))
}

async fn update_me(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Body(body): Body<ModifyCurrentUserRequest>,
) -> Result<Json<UserDto>, ApiError> {
    let input = UpdateCurrentUserInput {
        username: body.username,
        global_name: body.global_name,
        avatar: images::patch(body.avatar)?,
    };
    let user = update_current_user::execute(&state.ports, &actor, input).await?;
    let locale = state.ports.users.settings(user.id).await?.locale;
    Ok(Json(UserDto::private(&user, &locale)))
}

async fn public_user(
    State(state): State<AppState>,
    Caller(_actor): Caller,
    Path(id): Path<String>,
) -> Result<Json<UserDto>, ApiError> {
    let user = get_user::execute(&state.ports, parse_id(&id)?).await?;
    Ok(Json(UserDto::public(&user)))
}

async fn settings(
    State(state): State<AppState>,
    Caller(actor): Caller,
) -> Result<Json<UserSettingsDto>, ApiError> {
    let stored = get_settings::execute(&state.ports, &actor).await?;
    Ok(Json(UserSettingsDto::from(&stored)))
}

fn custom_status(
    raw: Option<Option<contracts::requests::CustomStatusRequest>>,
) -> Option<Option<CustomStatus>> {
    raw.map(|inner| {
        inner.map(|status| CustomStatus {
            text: status.text,
            emoji_name: status.emoji_name,
            expires_at: status.expires_at,
        })
    })
}

async fn update_settings_handler(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Body(body): Body<ModifyUserSettingsRequest>,
) -> Result<Json<UserSettingsDto>, ApiError> {
    let input = UpdateSettingsInput {
        default_delivery_mode: body.default_delivery_mode,
        e2ee_calls_default: body.e2ee_calls_default,
        theme: body.theme,
        locale: body.locale,
        status: body.status,
        custom_status: custom_status(body.custom_status),
        developer_mode: body.developer_mode,
        accent_color: body.accent_color,
    };
    let stored = update_settings::execute(&state.ports, &actor, input).await?;
    Ok(Json(UserSettingsDto::from(&stored)))
}

async fn private_channels(
    State(state): State<AppState>,
    Caller(actor): Caller,
) -> Result<Json<Vec<ChannelDto>>, ApiError> {
    let views = list_private_channels::execute(&state.ports, &actor).await?;
    Ok(Json(
        views
            .iter()
            .map(|view| {
                ChannelDto::private(
                    &view.channel,
                    &view.recipients,
                    view.delivery_mode,
                    Some(actor.user_id),
                )
            })
            .collect(),
    ))
}

async fn open_dm(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Body(body): Body<CreateDmRequest>,
) -> Result<Json<ChannelDto>, ApiError> {
    let recipients: Vec<Snowflake> = body.recipient_ids.into_iter().map(|Id(id)| id).collect();
    let view = open_private_channel::execute(&state.ports, &actor, recipients).await?;
    Ok(Json(ChannelDto::private(
        &view.channel,
        &view.recipients,
        view.delivery_mode,
        Some(actor.user_id),
    )))
}

async fn delivery_mode(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(channel_id): Path<String>,
    Body(body): Body<SetDeliveryModeRequest>,
) -> Result<Json<ChannelDto>, ApiError> {
    let mode = body
        .mode
        .as_deref()
        .map(|raw| {
            raw.parse::<DeliveryMode>().map_err(|_| {
                AppError::invalid("mode", "INVALID_DELIVERY_MODE", "Unknown delivery mode.")
            })
        })
        .transpose()?;
    let view =
        set_delivery_override::execute(&state.ports, &actor, parse_id(&channel_id)?, mode).await?;
    Ok(Json(ChannelDto::private(
        &view.channel,
        &view.recipients,
        view.delivery_mode,
        Some(actor.user_id),
    )))
}

async fn relationships(
    State(state): State<AppState>,
    Caller(actor): Caller,
) -> Result<Json<Vec<RelationshipDto>>, ApiError> {
    let rows = list_relationships::execute(&state.ports, &actor).await?;
    Ok(Json(
        rows.iter()
            .map(|(relationship, user)| RelationshipDto::new(relationship, user))
            .collect(),
    ))
}

#[derive(serde::Deserialize)]
struct FriendRequest {
    username: String,
}

async fn friend_request(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Body(body): Body<FriendRequest>,
) -> Result<StatusCode, ApiError> {
    send_friend_request::execute(&state.ports, &actor, &body.username).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn put_relationship_handler(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(target): Path<String>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let kind = if body.is_empty() {
        None
    } else {
        serde_json::from_slice::<PutRelationshipRequest>(&body)
            .map_err(|error| AppError::invalid("type", "INVALID_BODY", error.to_string()))?
            .kind
    };
    put_relationship::execute(&state.ports, &actor, parse_id(&target)?, kind).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_relationship(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(target): Path<String>,
) -> Result<StatusCode, ApiError> {
    remove_relationship::execute(&state.ports, &actor, parse_id(&target)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/users/@me",
            get(current_user)
                .patch(update_me)
                .layer(DefaultBodyLimit::max(IMAGE_BODY_LIMIT)),
        )
        .route(
            "/users/@me/settings",
            get(settings).patch(update_settings_handler),
        )
        .route("/users/@me/channels", get(private_channels).post(open_dm))
        .route("/users/@me/channels/{id}/delivery-mode", put(delivery_mode))
        .route(
            "/users/@me/relationships",
            get(relationships).post(friend_request),
        )
        .route(
            "/users/@me/relationships/{id}",
            put(put_relationship_handler).delete(delete_relationship),
        )
        .route("/users/{id}", get(public_user))
}
