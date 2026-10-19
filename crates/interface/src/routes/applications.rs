//! Applications, bot accounts, application commands, OAuth2 bot authorization
//! and interactions (P9).

use axum::extract::DefaultBodyLimit;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use application::use_cases::applications::authorize_bot::{self, AuthorizeBotInput};
use application::use_cases::applications::create_application::{self, CreateApplicationInput};
use application::use_cases::applications::get_application;
use application::use_cases::applications::get_authorize_info;
use application::use_cases::applications::list_applications;
use application::use_cases::applications::reset_bot_token;
use application::use_cases::applications::update_application::{self, UpdateApplicationInput};
use application::use_cases::commands::command_rules::CommandInput;
use application::use_cases::commands::guild_command_index;
use application::use_cases::commands::{
    create_command, delete_command, get_command, list_commands, overwrite_commands, update_command,
};
use application::use_cases::interactions::create_interaction::{self, CreateInteractionInput};
use application::use_cases::interactions::respond_to_interaction::{self, InteractionResponse};
use application::Resource;
use contracts::objects::{
    ApplicationCommandDto, ApplicationDto, CommandIndexDto, GuildDto, UserDto,
};
use contracts::requests::{
    ApplicationCommandRequest, CreateApplicationRequest, CreateInteractionRequest,
    InteractionResponseRequest, ModifyApplicationRequest, OAuth2AuthorizeRequest,
};
use contracts::wire::{Id, Perm};
use domain::Snowflake;

use crate::error::ApiError;
use crate::extract::{Body, Caller};
use crate::images::{self, IMAGE_BODY_LIMIT};
use crate::routes::ids::snowflake;
use crate::state::AppState;

fn region(state: &AppState) -> &str {
    &state.ports.settings.default_voice_region
}

async fn create(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Body(body): Body<CreateApplicationRequest>,
) -> Result<(StatusCode, Json<ApplicationDto>), ApiError> {
    let view = create_application::execute(
        &state.ports,
        &actor,
        CreateApplicationInput {
            name: body.name,
            description: body.description,
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(ApplicationDto::new(
            &view.application,
            &view.bot,
            &view.owner,
        )),
    ))
}

async fn list(
    State(state): State<AppState>,
    Caller(actor): Caller,
) -> Result<Json<Vec<ApplicationDto>>, ApiError> {
    let views = list_applications::execute(&state.ports, &actor).await?;
    Ok(Json(
        views
            .iter()
            .map(|view| ApplicationDto::new(&view.application, &view.bot, &view.owner))
            .collect(),
    ))
}

async fn fetch(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
) -> Result<Json<ApplicationDto>, ApiError> {
    let view =
        get_application::execute(&state.ports, &actor, snowflake(&id, Resource::Application)?)
            .await?;
    Ok(Json(ApplicationDto::new(
        &view.application,
        &view.bot,
        &view.owner,
    )))
}

async fn modify(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
    Body(body): Body<ModifyApplicationRequest>,
) -> Result<Json<ApplicationDto>, ApiError> {
    let input = UpdateApplicationInput {
        name: body.name,
        description: body.description,
        icon: images::patch(body.icon)?,
        bot_public: body.bot_public,
        bot_require_code_grant: body.bot_require_code_grant,
        flags: body.flags,
        interactions_endpoint_url: body.interactions_endpoint_url,
    };
    let view = update_application::execute(
        &state.ports,
        &actor,
        snowflake(&id, Resource::Application)?,
        input,
    )
    .await?;
    Ok(Json(ApplicationDto::new(
        &view.application,
        &view.bot,
        &view.owner,
    )))
}

async fn reset_token(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let token =
        reset_bot_token::execute(&state.ports, &actor, snowflake(&id, Resource::Application)?)
            .await?;
    Ok(Json(json!({ "token": token })))
}

fn command_input(body: ApplicationCommandRequest) -> CommandInput {
    CommandInput {
        name: body.name,
        description: body.description,
        kind: body.kind,
        options: body.options,
        default_member_permissions: body
            .default_member_permissions
            .map(|inner| inner.map(|Perm(p)| p)),
        dm_permission: body.dm_permission,
        nsfw: body.nsfw,
    }
}

fn scope(raw: Option<&str>) -> Result<Option<Snowflake>, ApiError> {
    raw.map(|guild| snowflake(guild, Resource::Guild))
        .transpose()
}

#[derive(Deserialize)]
struct CommandPath {
    id: String,
    guild_id: Option<String>,
}

async fn commands(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(path): Path<CommandPath>,
) -> Result<Json<Vec<ApplicationCommandDto>>, ApiError> {
    let found = list_commands::execute(
        &state.ports,
        &actor,
        snowflake(&path.id, Resource::Application)?,
        scope(path.guild_id.as_deref())?,
    )
    .await?;
    Ok(Json(
        found.iter().map(ApplicationCommandDto::from).collect(),
    ))
}

#[derive(Deserialize)]
struct GuildPath {
    id: String,
}

async fn command_index(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(path): Path<GuildPath>,
) -> Result<Json<CommandIndexDto>, ApiError> {
    let index =
        guild_command_index::execute(&state.ports, &actor, snowflake(&path.id, Resource::Guild)?)
            .await?;
    Ok(Json(CommandIndexDto::new(
        &index.applications,
        &index.commands,
    )))
}

#[derive(Deserialize)]
struct CommandItemPath {
    id: String,
    guild_id: Option<String>,
    command_id: String,
}

async fn create_one(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(path): Path<CommandPath>,
    Body(body): Body<ApplicationCommandRequest>,
) -> Result<(StatusCode, Json<ApplicationCommandDto>), ApiError> {
    let (command, created) = create_command::execute(
        &state.ports,
        &actor,
        snowflake(&path.id, Resource::Application)?,
        scope(path.guild_id.as_deref())?,
        command_input(body),
    )
    .await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(ApplicationCommandDto::from(&command))))
}

async fn overwrite(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(path): Path<CommandPath>,
    Body(body): Body<Vec<ApplicationCommandRequest>>,
) -> Result<Json<Vec<ApplicationCommandDto>>, ApiError> {
    let found = overwrite_commands::execute(
        &state.ports,
        &actor,
        snowflake(&path.id, Resource::Application)?,
        scope(path.guild_id.as_deref())?,
        body.into_iter().map(command_input).collect(),
    )
    .await?;
    Ok(Json(
        found.iter().map(ApplicationCommandDto::from).collect(),
    ))
}

async fn command_one(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(path): Path<CommandItemPath>,
) -> Result<Json<ApplicationCommandDto>, ApiError> {
    let command = get_command::execute(
        &state.ports,
        &actor,
        snowflake(&path.id, Resource::Application)?,
        scope(path.guild_id.as_deref())?,
        snowflake(&path.command_id, Resource::Command)?,
    )
    .await?;
    Ok(Json(ApplicationCommandDto::from(&command)))
}

async fn command_modify(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(path): Path<CommandItemPath>,
    Body(body): Body<ApplicationCommandRequest>,
) -> Result<Json<ApplicationCommandDto>, ApiError> {
    let command = update_command::execute(
        &state.ports,
        &actor,
        snowflake(&path.id, Resource::Application)?,
        scope(path.guild_id.as_deref())?,
        snowflake(&path.command_id, Resource::Command)?,
        command_input(body),
    )
    .await?;
    Ok(Json(ApplicationCommandDto::from(&command)))
}

async fn command_remove(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(path): Path<CommandItemPath>,
) -> Result<StatusCode, ApiError> {
    delete_command::execute(
        &state.ports,
        &actor,
        snowflake(&path.id, Resource::Application)?,
        scope(path.guild_id.as_deref())?,
        snowflake(&path.command_id, Resource::Command)?,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct AuthorizeQuery {
    client_id: String,
}

#[derive(Serialize)]
pub struct AuthorizeInfoDto {
    application: ApplicationDto,
    user: UserDto,
    guilds: Vec<GuildDto>,
}

async fn authorize_info(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Query(query): Query<AuthorizeQuery>,
) -> Result<Json<AuthorizeInfoDto>, ApiError> {
    let info = get_authorize_info::execute(
        &state.ports,
        &actor,
        snowflake(&query.client_id, Resource::Application)?,
    )
    .await?;
    let region = region(&state);
    Ok(Json(AuthorizeInfoDto {
        application: ApplicationDto::new(
            &info.application.application,
            &info.application.bot,
            &info.application.owner,
        ),
        user: UserDto::public(&info.user),
        guilds: info
            .guilds
            .iter()
            .map(|(guild, permissions)| GuildDto::partial(guild, *permissions, region))
            .collect(),
    }))
}

async fn authorize(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Body(body): Body<OAuth2AuthorizeRequest>,
) -> Result<StatusCode, ApiError> {
    let input = AuthorizeBotInput {
        client_id: body.client_id.0,
        guild_id: body.guild_id.0,
        permissions: body.permissions.0,
        scope: body.scope,
    };
    authorize_bot::execute(&state.ports, &actor, input).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_interaction_handler(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Body(body): Body<CreateInteractionRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let input = CreateInteractionInput {
        application_id: body.application_id.0,
        kind: body.kind,
        channel_id: body.channel_id.0,
        data: body.data,
        source_message_id: body.message_id.map(|Id(id)| id),
    };
    let id = create_interaction::execute(&state.ports, &actor, input).await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id.to_string() }))))
}

async fn callback(
    State(state): State<AppState>,
    Path((interaction_id, token)): Path<(String, String)>,
    Body(body): Body<InteractionResponseRequest>,
) -> Result<StatusCode, ApiError> {
    let response = InteractionResponse {
        kind: body.kind,
        data: body.data,
    };
    respond_to_interaction::execute(
        &state.ports,
        snowflake(&interaction_id, Resource::Interaction)?,
        &token,
        response,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/applications", get(list).post(create))
        .route(
            "/applications/{id}",
            get(fetch)
                .patch(modify)
                .layer(DefaultBodyLimit::max(IMAGE_BODY_LIMIT)),
        )
        .route("/applications/{id}/bot/reset-token", post(reset_token))
        .route(
            "/applications/{id}/commands",
            get(commands).put(overwrite).post(create_one),
        )
        .route(
            "/applications/{id}/commands/{command_id}",
            get(command_one)
                .patch(command_modify)
                .delete(command_remove),
        )
        .route(
            "/applications/{id}/guilds/{guild_id}/commands",
            get(commands).put(overwrite).post(create_one),
        )
        .route(
            "/applications/{id}/guilds/{guild_id}/commands/{command_id}",
            get(command_one)
                .patch(command_modify)
                .delete(command_remove),
        )
        .route("/guilds/{id}/application-command-index", get(command_index))
        .route("/oauth2/authorize", get(authorize_info).post(authorize))
        .route("/interactions", post(create_interaction_handler))
        .route("/interactions/{id}/{token}/callback", post(callback))
}
