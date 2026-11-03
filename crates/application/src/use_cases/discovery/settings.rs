use domain::discovery::{self, GuildDiscovery, Ineligibility};
use domain::permissions::Permissions;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::support::guild_context::GuildContext;

/// A guild's listing plus what currently stands in the way of listing it.
#[derive(Debug, Clone)]
pub struct DiscoverySettings {
    pub listing: Option<GuildDiscovery>,
    pub blockers: Vec<Ineligibility>,
    pub member_count: u64,
}

#[derive(Debug, Clone)]
pub struct UpdateDiscoveryInput {
    pub enabled: bool,
    pub category: Option<String>,
    pub tags: Vec<String>,
}

async fn describe(ports: &Ports, ctx: &GuildContext) -> AppResult<DiscoverySettings> {
    let guild_id = ctx.guild.id;
    let member_count = ports.guilds.member_count(guild_id).await?;
    let nsfw = ports.guilds.has_nsfw_channel(guild_id).await?;
    Ok(DiscoverySettings {
        listing: ports.guilds.discovery(guild_id).await?,
        blockers: discovery::ineligibilities(
            &ctx.guild,
            member_count,
            nsfw,
            ports.settings.discovery_rules(),
            ctx.now,
        ),
        member_count,
    })
}

/// `GET /guilds/{id}/discovery`; needs MANAGE_GUILD.
pub async fn get(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
) -> AppResult<DiscoverySettings> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_GUILD)?;
    describe(ports, &ctx).await
}

/// `PUT /guilds/{id}/discovery`; listing is refused while a rule is unmet.
pub async fn update(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    input: UpdateDiscoveryInput,
) -> AppResult<DiscoverySettings> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_GUILD)?;
    if !input.enabled {
        ports.guilds.set_discovery(guild_id, None).await?;
        return describe(ports, &ctx).await;
    }
    let category = discovery::category(input.category.as_deref().unwrap_or_default())?;
    let tags = discovery::tags(&input.tags)?;
    let state = describe(ports, &ctx).await?;
    if let Some(blocker) = state.blockers.first() {
        return Err(AppError::BadRequest {
            code: 50_035,
            message: format!("Guild is not eligible for discovery: {}.", blocker.code()),
        });
    }
    let listed_at = state.listing.map_or(ctx.now, |l| l.listed_at);
    ports
        .guilds
        .set_discovery(
            guild_id,
            Some(&GuildDiscovery {
                category,
                tags,
                listed_at,
            }),
        )
        .await?;
    describe(ports, &ctx).await
}
