//! Invites (P9): create per channel, read, accept, revoke and list per guild.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};

use application::support::audit::AuditContext;
use application::use_cases::invites::create_invite::CreateInviteInput;
use application::use_cases::invites::get_invite::InviteView;
use application::use_cases::invites::{
    accept_invite, create_invite, delete_invite, get_invite, list_guild_invites,
};
use application::Resource;
use contracts::objects::InviteDto;
use contracts::requests::CreateInviteRequest;

use crate::error::ApiError;
use crate::extract::{AuditReason, Body, Caller};
use crate::routes::ids::snowflake;
use crate::state::AppState;

fn to_dto(view: &InviteView) -> InviteDto {
    InviteDto::new(
        &view.invite,
        &view.guild,
        &view.channel,
        view.inviter.as_ref(),
        view.member_count,
    )
}

async fn create(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path(channel): Path<String>,
    Body(body): Body<CreateInviteRequest>,
) -> Result<(StatusCode, Json<InviteDto>), ApiError> {
    let input = CreateInviteInput {
        max_age: body.max_age,
        max_uses: body.max_uses,
        temporary: body.temporary,
    };
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    let view = create_invite::execute(
        &state.ports,
        &actor,
        snowflake(&channel, Resource::Channel)?,
        input,
        audit,
    )
    .await?;
    Ok((StatusCode::OK, Json(to_dto(&view))))
}

async fn fetch(
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> Result<Json<InviteDto>, ApiError> {
    let view = get_invite::execute(&state.ports, &code).await?;
    Ok(Json(to_dto(&view)))
}

async fn accept(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(code): Path<String>,
) -> Result<Json<InviteDto>, ApiError> {
    let view = accept_invite::execute(&state.ports, &actor, &code).await?;
    Ok(Json(to_dto(&view)))
}

async fn revoke(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path(code): Path<String>,
) -> Result<Json<InviteDto>, ApiError> {
    let audit = AuditContext {
        actor_id: actor.user_id,
        reason: reason.as_deref(),
    };
    let view = delete_invite::execute(&state.ports, &actor, &code, audit).await?;
    Ok(Json(to_dto(&view)))
}

async fn guild_invites(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(guild): Path<String>,
) -> Result<Json<Vec<InviteDto>>, ApiError> {
    let views =
        list_guild_invites::execute(&state.ports, &actor, snowflake(&guild, Resource::Guild)?)
            .await?;
    Ok(Json(views.iter().map(to_dto).collect()))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/channels/{id}/invites", post(create))
        .route("/invites/{code}", get(fetch).post(accept).delete(revoke))
        .route("/guilds/{id}/invites", get(guild_invites))
}
