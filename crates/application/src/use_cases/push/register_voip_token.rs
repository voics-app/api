use domain::push_token::{
    validate_token, PushEnvironment, PushPlatform, PushTokenError, VoipToken,
};

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct RegisterVoipTokenInput {
    pub token: String,
    pub environment: PushEnvironment,
}

/// Stores the PushKit token of the caller's session, replacing the one it registered before. The
/// app calls this every time PushKit reports its token, which is on every start. The token is
/// independent of the notification token: it works when notifications are switched off.
pub async fn execute(ports: &Ports, actor: &Actor, input: RegisterVoipTokenInput) -> AppResult<()> {
    actor.require_user()?;
    let session_id = actor.session_id().ok_or(AppError::Unauthorized)?;
    validate_token(PushPlatform::Apns, &input.token).map_err(|error| {
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
    ports
        .sessions
        .find(session_id)
        .await?
        .filter(|session| session.revoked_at.is_none())
        .ok_or(AppError::Unauthorized)?;
    let now = ports.clock.now();
    ports
        .push_tokens
        .upsert_voip(&VoipToken {
            session_id,
            user_id: actor.user_id,
            token: input.token,
            environment: input.environment,
            created_at: now,
            updated_at: now,
        })
        .await
}
