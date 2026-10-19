use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::use_cases::applications::app_access::{view, ApplicationView};

pub async fn execute(ports: &Ports, actor: &Actor) -> AppResult<Vec<ApplicationView>> {
    actor.require_user()?;
    let mut views = Vec::new();
    for application in ports.applications.list_for_owner(actor.user_id).await? {
        views.push(view(ports, application).await?);
    }
    Ok(views)
}
