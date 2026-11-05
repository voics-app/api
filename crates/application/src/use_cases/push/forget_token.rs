use domain::push_token::PushPlatform;

use crate::context::Ports;
use crate::error::AppResult;

/// `DELETE /internal/v1/push-tokens`: the notifier reports a token the provider says is dead
/// (APNs 410 or BadDeviceToken). Returns whether a row was removed.
pub async fn execute(ports: &Ports, platform: PushPlatform, token: &str) -> AppResult<bool> {
    ports.push_tokens.delete_by_token(platform, token).await
}
