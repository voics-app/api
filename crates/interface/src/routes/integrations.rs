use crate::{
    error::ApiError,
    extract::{AuditReason, Caller},
    routes::ids::snowflake,
    state::AppState,
};
use application::{
    support::audit::AuditContext,
    use_cases::applications::{consents, integrations},
    Resource,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use contracts::{
    integrations::{ApplicationAuthorizationDto, AuthorizationGuildDto, GuildIntegrationDto},
    objects::ApplicationDto,
    wire::{Id, Perm},
};

async fn authorizations(
    State(state): State<AppState>,
    Caller(actor): Caller,
) -> Result<Json<Vec<ApplicationAuthorizationDto>>, ApiError> {
    let views = consents::list(&state.ports, &actor).await?;
    Ok(Json(
        views
            .into_iter()
            .map(|v| ApplicationAuthorizationDto {
                id: Id(v.authorization.id),
                application: ApplicationDto::new(
                    &v.application.application,
                    &v.application.bot,
                    &v.application.owner,
                ),
                guild: AuthorizationGuildDto {
                    id: Id(v.guild.id),
                    name: v.guild.name,
                    icon: v.guild.icon,
                },
                scopes: v.authorization.scopes,
                permissions: Perm(v.authorization.permissions),
                created_at: v.authorization.created_at,
                authorized_at: v.authorization.authorized_at,
            })
            .collect(),
    ))
}

async fn revoke(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    consents::revoke(&state.ports, &actor, snowflake(&id, Resource::Application)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn installed(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(guild): Path<String>,
) -> Result<Json<Vec<GuildIntegrationDto>>, ApiError> {
    let views =
        integrations::list(&state.ports, &actor, snowflake(&guild, Resource::Guild)?).await?;
    Ok(Json(
        views
            .into_iter()
            .map(|v| GuildIntegrationDto {
                application: ApplicationDto::new(
                    &v.application.application,
                    &v.application.bot,
                    &v.application.owner,
                ),
                permissions: Perm(v.permissions),
                installed_at: v.installed_at,
            })
            .collect(),
    ))
}

async fn remove(
    State(state): State<AppState>,
    Caller(actor): Caller,
    AuditReason(reason): AuditReason,
    Path((guild, app)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    integrations::remove(
        &state.ports,
        &actor,
        snowflake(&guild, Resource::Guild)?,
        snowflake(&app, Resource::Application)?,
        AuditContext {
            actor_id: actor.user_id,
            reason: reason.as_deref(),
        },
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/oauth2/tokens", get(authorizations))
        .route("/oauth2/tokens/{id}", axum::routing::delete(revoke))
        .route("/guilds/{id}/integrations", get(installed))
        .route(
            "/guilds/{id}/integrations/{application_id}",
            axum::routing::delete(remove),
        )
}
