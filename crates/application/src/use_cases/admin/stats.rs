use chrono::Duration;

use crate::context::Ports;
use crate::error::AppResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiStats {
    pub users_total: u64,
    pub guilds_total: u64,
    pub channels_total: u64,
    pub bots_total: u64,
    pub signups_24h: u64,
}

/// `GET /internal/v1/admin/stats`. Counts only.
pub async fn execute(ports: &Ports) -> AppResult<ApiStats> {
    let users = ports
        .users
        .stats(ports.clock.now() - Duration::hours(24))
        .await?;
    Ok(ApiStats {
        users_total: users.users_total,
        guilds_total: ports.guilds.count().await?,
        channels_total: ports.channels.count().await?,
        bots_total: users.bots_total,
        signups_24h: users.signups_since,
    })
}
