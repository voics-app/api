use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::use_cases::applications::app_access::require_owner;
use crate::use_cases::auth::authenticate::encode_bot_token_prefix;
use crate::write_set::{Mutation, WriteSet};

/// Issues a new bot token (the old one stops working). Returned once only.
pub async fn execute(ports: &Ports, actor: &Actor, application_id: Snowflake) -> AppResult<String> {
    let application = require_owner(ports, actor, application_id).await?;
    let token = format!("{}.{}", encode_bot_token_prefix(application.bot_user_id), ports.secrets.token(32));
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::SetBotToken {
        application_id,
        bot_user_id: application.bot_user_id,
        token_hash: ports.secrets.hash(&token),
    });
    ports.unit_of_work.commit(writes).await?;
    Ok(token)
}
