//! Members, roles and bans of a guild (P9).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;

use application::support::audit::AuditContext;
use application::use_cases::bans::{create_ban, get_ban, list_bans, remove_ban};
use application::use_cases::members::update_member::UpdateMemberInput;
use application::use_cases::members::{
    add_member_role, get_member, list_members, remove_member, remove_member_role, update_member,
};
use application::use_cases::roles::create_role::RoleInput;
use application::use_cases::roles::{
    create_role, delete_role, list_roles, reorder_roles, update_role,
};
use application::Resource;
use contracts::objects::{BanDto, MemberDto, RoleDto};
use contracts::requests::{ModifyMemberRequest, RolePositionRequest, RoleRequest};
use contracts::wire::Id;

use crate::error::ApiError;
use crate::extract::{AuditReason, Body, Caller};
use crate::routes::ids::snowflake;
use crate::state::AppState;

#[derive(Deserialize)]
struct MemberQuery {
    limit: Option<u32>,
    after: Option<String>,
}

async fn list_all(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(guild): Path<String>,
    Query(params): Query<MemberQuery>,
) -> Result<Json<Vec<MemberDto>>, ApiError> {
    let after = params
        .after
        .as_deref()
        .map(|raw| snowflake(raw, Resource::Member))
        .transpose()?;
    let members = list_members::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        params.limit.map(|limit| limit.clamp(1, 1000)),
        after,
    )
    .await?;
    Ok(Json(members.iter().map(MemberDto::from).collect()))
}

async fn fetch_member(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path((guild, user)): Path<(String, String)>,
) -> Result<Json<MemberDto>, ApiError> {
    let profile = get_member::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&user, Resource::Member)?,
    )
    .await?;
    Ok(Json(MemberDto::from(&profile)))
}

async fn modify_member(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path((guild, user)): Path<(String, String)>,
    Body(body): Body<ModifyMemberRequest>,
) -> Result<Json<MemberDto>, ApiError> {
    let input = UpdateMemberInput {
        nick: body.nick,
        roles: body
            .roles
            .map(|roles| roles.into_iter().map(|Id(id)| id).collect()),
        mute: body.mute,
        deaf: body.deaf,
        communication_disabled_until: body.communication_disabled_until,
        voice_channel: body.channel_id.map(|channel| channel.map(|Id(id)| id)),
    };
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    let profile = update_member::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&user, Resource::Member)?,
        input,
        audit,
    )
    .await?;
    Ok(Json(MemberDto::from(&profile)))
}

async fn kick(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path((guild, user)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    remove_member::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&user, Resource::Member)?,
        audit,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn grant_role(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path((guild, user, role)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    add_member_role::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&user, Resource::Member)?,
        snowflake(&role, Resource::Role)?,
        audit,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_role(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path((guild, user, role)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    remove_member_role::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&user, Resource::Member)?,
        snowflake(&role, Resource::Role)?,
        audit,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn roles(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(guild): Path<String>,
) -> Result<Json<Vec<RoleDto>>, ApiError> {
    let found =
        list_roles::execute(&state.ports, &actor, snowflake(&guild, Resource::Guild)?).await?;
    Ok(Json(found.iter().map(RoleDto::from).collect()))
}

fn role_input(body: RoleRequest) -> RoleInput {
    RoleInput {
        name: body.name,
        permissions: body.permissions.map(|contracts::wire::Perm(p)| p),
        color: body.color,
        hoist: body.hoist,
        mentionable: body.mentionable,
        unicode_emoji: body.unicode_emoji,
    }
}

async fn add_role(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path(guild): Path<String>,
    Body(body): Body<RoleRequest>,
) -> Result<(StatusCode, Json<RoleDto>), ApiError> {
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    let role = create_role::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        role_input(body),
        audit,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(RoleDto::from(&role))))
}

async fn modify_role(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path((guild, role)): Path<(String, String)>,
    Body(body): Body<RoleRequest>,
) -> Result<Json<RoleDto>, ApiError> {
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    let updated = update_role::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&role, Resource::Role)?,
        role_input(body),
        audit,
    )
    .await?;
    Ok(Json(RoleDto::from(&updated)))
}

async fn remove_role(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path((guild, role)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    delete_role::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&role, Resource::Role)?,
        audit,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reorder_role_list(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(guild): Path<String>,
    Body(body): Body<Vec<RolePositionRequest>>,
) -> Result<Json<Vec<RoleDto>>, ApiError> {
    let positions = body
        .into_iter()
        .map(|item| (item.id.0, item.position))
        .collect();
    let updated = reorder_roles::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        positions,
    )
    .await?;
    Ok(Json(updated.iter().map(RoleDto::from).collect()))
}

async fn bans(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(guild): Path<String>,
) -> Result<Json<Vec<BanDto>>, ApiError> {
    let found =
        list_bans::execute(&state.ports, &actor, snowflake(&guild, Resource::Guild)?).await?;
    Ok(Json(found.iter().map(BanDto::from).collect()))
}

async fn fetch_ban(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path((guild, user)): Path<(String, String)>,
) -> Result<Json<BanDto>, ApiError> {
    let ban = get_ban::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&user, Resource::User)?,
    )
    .await?;
    Ok(Json(BanDto::from(&ban)))
}

async fn ban(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path((guild, user)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    create_ban::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&user, Resource::User)?,
        audit,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unban(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path((guild, user)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    remove_ban::execute(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&user, Resource::User)?,
        audit,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/guilds/{id}/members", get(list_all))
        .route(
            "/guilds/{id}/members/{user_id}",
            get(fetch_member).patch(modify_member).delete(kick),
        )
        .route(
            "/guilds/{id}/members/{user_id}/roles/{role_id}",
            put(grant_role).delete(revoke_role),
        )
        .route(
            "/guilds/{id}/roles",
            get(roles).post(add_role).patch(reorder_role_list),
        )
        .route(
            "/guilds/{id}/roles/{role_id}",
            axum::routing::patch(modify_role).delete(remove_role),
        )
        .route("/guilds/{id}/bans", get(bans))
        .route(
            "/guilds/{id}/bans/{user_id}",
            get(fetch_ban).put(ban).delete(unban),
        )
}
