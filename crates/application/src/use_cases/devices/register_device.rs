use domain::device::{Device, IDENTITY_KEY_LENGTH, MAX_DEVICES_PER_USER};

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::use_cases::auth::tokens::access_token;

#[derive(Debug, Clone)]
pub struct RegisterDeviceInput {
    pub name: String,
    pub identity_key: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct RegisteredDevice {
    pub device: Device,
    /// A fresh access token whose `did` claim names the new device.
    pub access_token: String,
}

/// Registers a device and binds it to the caller's session.
pub async fn execute(ports: &Ports, actor: &Actor, input: RegisterDeviceInput) -> AppResult<RegisteredDevice> {
    actor.require_user()?;
    let session_id = actor.session_id().ok_or(AppError::Unauthorized)?;
    let name = input.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(AppError::invalid("name", "BASE_TYPE_BAD_LENGTH", "Must be between 1 and 64 in length."));
    }
    let identity_key: [u8; IDENTITY_KEY_LENGTH] = input.identity_key.try_into().map_err(|_| {
        AppError::invalid("identity_key_b64", "IDENTITY_KEY_INVALID", "Must be a 32-byte Ed25519 public key.")
    })?;
    if ports.devices.list_for_user(actor.user_id).await?.len() >= MAX_DEVICES_PER_USER {
        return Err(AppError::BadRequest { code: 30000, message: "Maximum number of devices reached.".into() });
    }
    let device = Device {
        id: ports.ids.next_id(),
        user_id: actor.user_id,
        name,
        identity_key,
        created_at: ports.clock.now(),
    };
    ports.devices.create(&device).await?;
    ports.sessions.bind_device(session_id, device.id).await?;
    let access_token = access_token(ports, actor.user_id, session_id, Some(device.id))?;
    Ok(RegisteredDevice { device, access_token })
}
