use domain::application::Webhook;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::use_cases::webhooks::webhook_access::require_manager;

pub async fn execute(ports: &Ports, actor: &Actor, webhook_id: Snowflake) -> AppResult<Webhook> {
    Ok(require_manager(ports, actor, webhook_id).await?.0)
}
