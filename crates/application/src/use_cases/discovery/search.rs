use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::ports::{DiscoveryEntry, DiscoveryQuery};

pub const MAX_LIMIT: u32 = 50;
pub const DEFAULT_LIMIT: u32 = 24;

#[derive(Debug, Clone, Default)]
pub struct SearchInput {
    pub text: Option<String>,
    pub category: Option<String>,
    pub tag: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct DiscoveryResults {
    pub entries: Vec<DiscoveryEntry>,
    /// Online members per entry, `None` when presence is unavailable.
    pub online: Vec<Option<u64>>,
    pub total: u64,
}

/// Members counted for the online figure; larger guilds report a lower bound.
const PRESENCE_SAMPLE: u32 = 10_000;

fn normalised(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_lowercase())
        .filter(|v| !v.is_empty())
}

/// `GET /discovery/guilds`: any signed-in user may browse.
pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    input: SearchInput,
) -> AppResult<DiscoveryResults> {
    actor.require_user()?;
    let rules = ports.settings.discovery_rules();
    let query = DiscoveryQuery {
        text: input
            .text
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty()),
        category: normalised(input.category),
        tag: normalised(input.tag),
        min_members: rules.min_members,
        created_before: ports.clock.now() - rules.min_age,
        limit: input.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT),
        offset: input.offset.unwrap_or(0),
    };
    let page = ports.guilds.search_discoverable(&query).await?;
    let mut online = Vec::with_capacity(page.entries.len());
    for entry in &page.entries {
        online.push(online_count(ports, entry.guild.id).await);
    }
    Ok(DiscoveryResults {
        entries: page.entries,
        online,
        total: page.total,
    })
}

/// Best effort: a presence outage must not break browsing.
async fn online_count(ports: &Ports, guild_id: domain::Snowflake) -> Option<u64> {
    let ids = ports
        .guilds
        .member_ids(guild_id, PRESENCE_SAMPLE)
        .await
        .ok()?;
    ports.presence.online_count(&ids).await.ok()
}
