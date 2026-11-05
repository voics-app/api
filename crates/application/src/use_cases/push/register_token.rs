use domain::push_token::{
    validate_token, PushEnvironment, PushPlatform, PushToken, PushTokenError,
};

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct RegisterPushTokenInput {
    pub platform: PushPlatform,
    pub token: String,
    pub environment: PushEnvironment,
    pub previews: bool,
}

/// Stores the push token of the caller's session, replacing the one it registered before. The
/// app calls this on every start, so a token that iOS rotated is refreshed the same way.
pub async fn execute(ports: &Ports, actor: &Actor, input: RegisterPushTokenInput) -> AppResult<()> {
    actor.require_user()?;
    let session_id = actor.session_id().ok_or(AppError::Unauthorized)?;
    validate_token(input.platform, &input.token).map_err(|error| {
        AppError::invalid(
            "token",
            match error {
                PushTokenError::Empty => "PUSH_TOKEN_EMPTY",
                PushTokenError::TooLong => "PUSH_TOKEN_TOO_LONG",
                PushTokenError::NotHex => "PUSH_TOKEN_INVALID",
            },
            error.to_string(),
        )
    })?;
    let device_id = ports
        .sessions
        .find(session_id)
        .await?
        .filter(|session| session.revoked_at.is_none())
        .ok_or(AppError::Unauthorized)?
        .device_id;
    let now = ports.clock.now();
    ports
        .push_tokens
        .upsert(&PushToken {
            session_id,
            user_id: actor.user_id,
            device_id,
            platform: input.platform,
            token: input.token,
            environment: input.environment,
            previews: input.previews,
            created_at: now,
            updated_at: now,
        })
        .await
}
