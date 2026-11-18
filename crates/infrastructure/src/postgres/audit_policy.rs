use application::AppResult;
use domain::audit::AuditLogEntry;
use domain::audit_policy::AuditPolicy;
use domain::Snowflake;
use sqlx::PgConnection;

use super::error::db;

pub(super) async fn set(
    conn: &mut PgConnection,
    guild_id: Snowflake,
    policy: AuditPolicy,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE guilds SET audit_log_enabled = $2, audit_log_retention_days = $3 WHERE id = $1",
    )
    .bind(guild_id.as_i64())
    .bind(policy.enabled)
    .bind(policy.retention_days as i16)
    .execute(conn)
    .await
    .map_err(db)?;
    Ok(())
}

pub(super) async fn records(conn: &mut PgConnection, entry: &AuditLogEntry) -> AppResult<bool> {
    // Serializes with a policy change, preventing a concurrent action from
    // recording an entry after disabling has committed.
    let enabled: Option<bool> =
        sqlx::query_scalar("SELECT audit_log_enabled FROM guilds WHERE id = $1 FOR SHARE")
            .bind(entry.guild_id.as_i64())
            .fetch_optional(conn)
            .await
            .map_err(db)?;
    Ok(enabled.is_some_and(|enabled| enabled || entry.changes_audit_policy()))
}
