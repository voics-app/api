//! Initiation policy for guild-related private conversations.
use crate::context::Ports;
use crate::error::{AppError, AppResult, Denial};
use domain::relationship::RelationshipType;
use domain::Snowflake;

/// Existing one-to-one conversations and friendships are intentionally grandfathered.
/// With multiple shared guilds, one permitting DMs is enough. When there is no shared
/// guild, retain the existing direct-message behavior.
pub async fn require_initiation(
    ports: &Ports,
    sender: Snowflake,
    recipient: Snowflake,
) -> AppResult<()> {
    if sender == recipient {
        return Ok(());
    }
    if ports
        .relationships
        .get(recipient, sender)
        .await?
        .map(|r| r.kind)
        == Some(RelationshipType::Blocked)
    {
        return Err(AppError::Forbidden(Denial::CannotMessageUser));
    }
    if ports
        .relationships
        .get(recipient, sender)
        .await?
        .map(|r| r.kind)
        == Some(RelationshipType::Friend)
        || ports.channels.find_dm(sender, recipient).await?.is_some()
    {
        return Ok(());
    }
    let settings = ports.users.settings(recipient).await?;
    let default = settings
        .preferences
        .get("default_guilds_restricted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let overrides = settings
        .preferences
        .get("guild_dm_restrictions")
        .and_then(|v| v.as_object());
    if !default && !overrides.is_some_and(|map| map.values().any(|v| v.as_bool() == Some(true))) {
        return Ok(());
    }
    let sender_guilds = ports.guilds.list_for_user(sender).await?;
    let recipient_guilds = ports.guilds.list_for_user(recipient).await?;
    let ids: std::collections::HashSet<_> = sender_guilds.into_iter().map(|g| g.id).collect();
    let shared: Vec<_> = recipient_guilds
        .into_iter()
        .filter(|g| ids.contains(&g.id))
        .collect();
    if shared.is_empty() {
        return Ok(());
    }
    if shared.iter().any(|g| {
        !overrides
            .and_then(|o| o.get(&g.id.to_string()))
            .and_then(|v| v.as_bool())
            .unwrap_or(default)
    }) {
        return Ok(());
    }
    Err(AppError::Forbidden(Denial::CannotMessageUser))
}
