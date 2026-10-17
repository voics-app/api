use domain::user::User;
use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;
use crate::support::views::require_user;

/// Public profile; the interface strips private fields.
pub async fn execute(ports: &Ports, user_id: Snowflake) -> AppResult<User> {
    require_user(ports, user_id).await
}
