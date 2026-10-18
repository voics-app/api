use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::use_cases::applications::app_access::{require_manager, view, ApplicationView};

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    application_id: Snowflake,
) -> AppResult<ApplicationView> {
    let application = require_manager(ports, actor, application_id).await?;
    view(ports, application).await
}
