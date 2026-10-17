use domain::audit::AuditLogEntry;
use domain::permissions::Permissions;
use domain::user::User;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::ports::AuditLogQuery;
use crate::support::guild_context::GuildContext;

#[derive(Debug, Clone)]
pub struct AuditLogPage {
    pub entries: Vec<AuditLogEntry>,
    pub users: Vec<User>,
}

pub async fn execute(ports: &Ports, actor: &Actor, guild_id: Snowflake, query: AuditLogQuery) -> AppResult<AuditLogPage> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::VIEW_AUDIT_LOG)?;
    let query = AuditLogQuery { limit: query.limit.clamp(1, 100), ..query };
    let entries = ports.audit_log.list(guild_id, query).await?;
    let mut user_ids: Vec<Snowflake> = entries.iter().flat_map(|e| [e.user_id, e.target_id]).flatten().collect();
    user_ids.sort();
    user_ids.dedup();
    let users = ports.users.find_many(&user_ids).await?;
    Ok(AuditLogPage { entries, users })
}
