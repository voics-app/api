//! Server discovery: `/discovery/*` browsing and `/guilds/{id}/discovery` settings (api#74).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use application::use_cases::discovery::join;
use application::use_cases::discovery::search::{self, SearchInput};
use application::use_cases::discovery::settings::{self, DiscoverySettings, UpdateDiscoveryInput};
use application::use_cases::reports::submit_report::{self, ReportTarget, SubmitReportInput};
use application::Resource;
use contracts::objects::{
    DiscoveryCategoriesDto, DiscoveryGuildDto, DiscoveryPageDto, DiscoverySettingsDto,
};
use contracts::requests::UpdateDiscoveryRequest;
use contracts::wire::Id;
use domain::discovery::CATEGORIES;

use crate::error::ApiError;
use crate::extract::{Body, Caller};
use crate::routes::ids::snowflake;
use crate::state::AppState;

#[derive(Deserialize)]
struct SearchQuery {
    query: Option<String>,
    category: Option<String>,
    tag: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
}

async fn browse(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Query(params): Query<SearchQuery>,
) -> Result<Json<DiscoveryPageDto>, ApiError> {
    let input = SearchInput {
        text: params.query,
        category: params.category,
        tag: params.tag,
        limit: params.limit,
        offset: params.offset,
    };
    let limit = input
        .limit
        .unwrap_or(search::DEFAULT_LIMIT)
        .clamp(1, search::MAX_LIMIT);
    let offset = input.offset.unwrap_or(0);
    let results = search::execute(&state.ports, &actor, input).await?;
    let guilds = results
        .entries
        .iter()
        .zip(&results.online)
        .map(|(entry, online)| DiscoveryGuildDto {
            id: Id(entry.guild.id),
            name: entry.guild.name.clone(),
            icon: entry.guild.icon.clone(),
            description: entry.guild.description.clone(),
            category: entry.discovery.category.clone(),
            tags: entry.discovery.tags.clone(),
            approximate_member_count: entry.member_count,
            approximate_presence_count: *online,
        })
        .collect();
    Ok(Json(DiscoveryPageDto {
        guilds,
        total: results.total,
        offset,
        limit,
    }))
}

async fn categories(Caller(_): Caller) -> Json<DiscoveryCategoriesDto> {
    Json(DiscoveryCategoriesDto {
        categories: CATEGORIES.to_vec(),
    })
}

async fn join_guild(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    join::execute(&state.ports, &actor, snowflake(&id, Resource::Guild)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportListingRequest {
    category: String,
    note: Option<String>,
}

async fn report_listing(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
    Body(body): Body<ReportListingRequest>,
) -> Result<StatusCode, ApiError> {
    submit_report::execute(
        &state.ports,
        &actor,
        SubmitReportInput {
            target: ReportTarget::Guild {
                guild_id: snowflake(&id, Resource::Guild)?,
            },
            category: body.category,
            note: body.note,
        },
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn dto(state: &DiscoverySettings) -> Json<DiscoverySettingsDto> {
    Json(DiscoverySettingsDto::new(
        state.listing.as_ref(),
        &state.blockers,
        state.member_count,
    ))
}

async fn settings_get(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
) -> Result<Json<DiscoverySettingsDto>, ApiError> {
    let current = settings::get(&state.ports, &actor, snowflake(&id, Resource::Guild)?).await?;
    Ok(dto(&current))
}

async fn settings_put(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
    Body(body): Body<UpdateDiscoveryRequest>,
) -> Result<Json<DiscoverySettingsDto>, ApiError> {
    let input = UpdateDiscoveryInput {
        enabled: body.enabled,
        category: body.category,
        tags: body.tags,
    };
    let updated = settings::update(
        &state.ports,
        &actor,
        snowflake(&id, Resource::Guild)?,
        input,
    )
    .await?;
    Ok(dto(&updated))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/discovery/guilds", get(browse))
        .route("/discovery/categories", get(categories))
        .route("/discovery/guilds/{id}/join", post(join_guild))
        .route("/discovery/guilds/{id}/reports", post(report_listing))
        .route(
            "/guilds/{id}/discovery",
            get(settings_get).put(settings_put),
        )
}
