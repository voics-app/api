//! Devices and MLS key packages (P9).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde::Serialize;

use application::use_cases::devices::register_device::RegisterDeviceInput;
use application::use_cases::devices::{
    claim_key_package, delete_device, list_user_devices, register_device, upload_key_packages,
};
use application::{AppError, Resource};
use contracts::objects::DeviceDto;
use contracts::requests::{CreateDeviceRequest, UploadKeyPackagesRequest};
use domain::Snowflake;

use crate::error::ApiError;
use crate::extract::{Body, Caller};
use crate::state::AppState;

fn parse_id(raw: &str) -> Result<Snowflake, ApiError> {
    raw.parse()
        .map_err(|_| AppError::NotFound(Resource::Device).into())
}

fn decode(field: &str, raw: &str) -> Result<Vec<u8>, AppError> {
    STANDARD
        .decode(raw)
        .map_err(|_| AppError::invalid(field, "INVALID_BASE64", "Value must be base64."))
}

#[derive(Serialize)]
pub struct RegisteredDeviceResponse {
    #[serde(flatten)]
    device: DeviceDto,
    access_token: String,
}

async fn register(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Body(body): Body<CreateDeviceRequest>,
) -> Result<(StatusCode, Json<RegisteredDeviceResponse>), ApiError> {
    let identity_key = decode("identity_key_b64", &body.identity_key_b64)?;
    let registered = register_device::execute(
        &state.ports,
        &actor,
        RegisterDeviceInput {
            name: body.name,
            identity_key,
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(RegisteredDeviceResponse {
            device: DeviceDto::from(&registered.device),
            access_token: registered.access_token,
        }),
    ))
}

async fn upload_packages(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(device_id): Path<String>,
    Body(body): Body<UploadKeyPackagesRequest>,
) -> Result<StatusCode, ApiError> {
    let packages = body
        .packages
        .iter()
        .map(|raw| decode("packages", raw))
        .collect::<Result<Vec<_>, _>>()?;
    upload_key_packages::execute(&state.ports, &actor, parse_id(&device_id)?, packages).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(device_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    delete_device::execute(&state.ports, &actor, parse_id(&device_id)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn user_devices(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path(user_id): Path<String>,
) -> Result<Json<Vec<DeviceDto>>, ApiError> {
    let devices = list_user_devices::execute(&state.ports, &actor, parse_id(&user_id)?).await?;
    Ok(Json(devices.iter().map(DeviceDto::from).collect()))
}

#[derive(Serialize)]
pub struct ClaimedPackage {
    package_b64: String,
}

async fn claim_package(
    State(state): State<AppState>,
    Caller(actor): Caller,
    Path((user_id, device_id)): Path<(String, String)>,
) -> Result<Json<ClaimedPackage>, ApiError> {
    let package = claim_key_package::execute(
        &state.ports,
        &actor,
        parse_id(&user_id)?,
        parse_id(&device_id)?,
    )
    .await?;
    Ok(Json(ClaimedPackage {
        package_b64: STANDARD.encode(package),
    }))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/devices", post(register))
        .route("/devices/{id}", delete(remove))
        .route("/devices/{id}/key-packages", post(upload_packages))
        .route("/users/{id}/devices", get(user_devices))
        .route(
            "/users/{id}/devices/{did}/key-packages/claim",
            post(claim_package),
        )
}
