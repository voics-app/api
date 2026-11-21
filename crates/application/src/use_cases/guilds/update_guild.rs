use serde_json::json;

use domain::audit::{AuditAction, AuditChange};
use domain::channel::ChannelType;
use domain::guild::{Guild, SYSTEM_CHANNEL_FLAGS_MASK};
use domain::permissions::Permissions;
use domain::validation;
use domain::Snowflake;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::guild_context::GuildContext;
use crate::support::images::{store_hashed, ImageUpload};
use crate::use_cases::guilds::create_guild::ensure_known_region;
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone, Default)]
pub struct UpdateGuildInput {
    pub features: Option<Vec<String>>,
    pub rules_channel_id: Option<Option<Snowflake>>,
    pub public_updates_channel_id: Option<Option<Snowflake>>,
    pub name: Option<String>,
    pub icon: Option<Option<ImageUpload>>,
    pub description: Option<Option<String>>,
    pub voice_region: Option<Option<String>>,
    pub verification_level: Option<u8>,
    pub default_message_notifications: Option<u8>,
    pub explicit_content_filter: Option<u8>,
    pub afk_channel_id: Option<Option<Snowflake>>,
    pub afk_timeout: Option<u32>,
    pub system_channel_id: Option<Option<Snowflake>>,
    pub system_channel_flags: Option<u32>,
    pub banner: Option<Option<ImageUpload>>,
    pub splash: Option<Option<ImageUpload>>,
    pub owner_id: Option<Snowflake>,
    pub prune_admin_only: Option<bool>,
    pub audit_log_enabled: Option<bool>,
    pub audit_log_retention_days: Option<u16>,
}

fn bounded(field: &str, value: u8, max: u8) -> AppResult<u8> {
    if value > max {
        return Err(AppError::invalid(
            field,
            "BASE_TYPE_CHOICES",
            format!("Must be between 0 and {max}."),
        ));
    }
    Ok(value)
}

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    input: UpdateGuildInput,
    audit: AuditContext<'_>,
) -> AppResult<Guild> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_GUILD)?;
    let before = ctx.guild.clone();
    let mut guild = ctx.guild.clone();
    let community_requested = super::community::apply(ports, &mut guild, &input).await?;
    if input.audit_log_enabled.is_some() || input.audit_log_retention_days.is_some() {
        ctx.require(Permissions::ADMINISTRATOR)?;
    }
    if let Some(enabled) = input.audit_log_enabled {
        guild.audit_policy.enabled = enabled;
    }
    if let Some(days) = input.audit_log_retention_days {
        guild.audit_policy.set_retention(days)?;
    }

    if let Some(name) = input.name {
        validation::guild_name(&name)?;
        guild.name = name.trim().to_owned();
    }
    if let Some(description) = input.description {
        if description
            .as_ref()
            .is_some_and(|d| d.chars().count() > 300)
        {
            return Err(AppError::invalid(
                "description",
                "BASE_TYPE_BAD_LENGTH",
                "Must be at most 300 characters.",
            ));
        }
        guild.description = description;
    }
    if let Some(region) = input.voice_region {
        if let Some(region) = &region {
            ensure_known_region(ports, region)?;
        }
        guild.voice_region = region;
    }
    if let Some(level) = input.verification_level {
        guild.verification_level = bounded("verification_level", level, 4)?;
    }
    if let Some(level) = input.default_message_notifications {
        guild.default_message_notifications = bounded("default_message_notifications", level, 1)?;
    }
    if let Some(level) = input.explicit_content_filter {
        guild.explicit_content_filter = bounded("explicit_content_filter", level, 2)?;
    }
    if let Some(admin_only) = input.prune_admin_only {
        guild.prune_admin_only = admin_only;
    }
    if let Some(timeout) = input.afk_timeout {
        if ![60, 300, 900, 1800, 3600].contains(&timeout) {
            return Err(AppError::invalid(
                "afk_timeout",
                "BASE_TYPE_CHOICES",
                "Invalid AFK timeout.",
            ));
        }
        guild.afk_timeout = timeout;
    }
    if let Some(afk) = input.afk_channel_id {
        if let Some(id) = afk {
            ensure_channel_kind(ports, guild_id, id, |k| k.is_voice(), "afk_channel_id").await?;
        }
        guild.afk_channel_id = afk;
    }
    if let Some(system) = input.system_channel_id {
        if let Some(id) = system {
            ensure_channel_kind(
                ports,
                guild_id,
                id,
                |k| k == ChannelType::GuildText,
                "system_channel_id",
            )
            .await?;
        }
        guild.system_channel_id = system;
    }
    if let Some(flags) = input.system_channel_flags {
        if flags & !SYSTEM_CHANNEL_FLAGS_MASK != 0 {
            return Err(AppError::invalid(
                "system_channel_flags",
                "BASE_TYPE_CHOICES",
                "Unknown system channel flags.",
            ));
        }
        guild.system_channel_flags = flags;
    }
    if let Some(owner_id) = input.owner_id.filter(|id| *id != guild.owner_id) {
        if !ctx.is_owner() {
            return Err(AppError::missing_permissions());
        }
        if ports.guilds.member(guild_id, owner_id).await?.is_none() {
            return Err(AppError::invalid(
                "owner_id",
                "GUILD_OWNER_NOT_MEMBER",
                "New owner must be a member.",
            ));
        }
        guild.owner_id = owner_id;
    }
    if let Some(icon) = input.icon {
        guild.icon = match icon {
            Some(image) => Some(store_hashed(ports, &format!("icons/{guild_id}"), image).await?),
            None => None,
        };
    }
    if let Some(banner) = input.banner {
        guild.banner = store_optional(ports, &format!("banners/{guild_id}"), banner).await?;
    }
    if let Some(splash) = input.splash {
        guild.splash = store_optional(ports, &format!("splashes/{guild_id}"), splash).await?;
    }

    let changes: Vec<AuditChange> = [
        AuditChange::if_changed(
            "features",
            json!(before
                .community
                .map(|_| vec!["COMMUNITY"])
                .unwrap_or_default()),
            json!(guild
                .community
                .map(|_| vec!["COMMUNITY"])
                .unwrap_or_default()),
        ),
        AuditChange::if_changed(
            "rules_channel_id",
            json!(before.community.map(|c| c.rules_channel_id.to_string())),
            json!(guild.community.map(|c| c.rules_channel_id.to_string())),
        ),
        AuditChange::if_changed(
            "public_updates_channel_id",
            json!(before
                .community
                .map(|c| c.public_updates_channel_id.to_string())),
            json!(guild
                .community
                .map(|c| c.public_updates_channel_id.to_string())),
        ),
        AuditChange::if_changed(
            "audit_log_enabled",
            json!(before.audit_policy.enabled),
            json!(guild.audit_policy.enabled),
        ),
        AuditChange::if_changed(
            "audit_log_retention_days",
            json!(before.audit_policy.retention_days),
            json!(guild.audit_policy.retention_days),
        ),
        AuditChange::if_changed("name", json!(before.name), json!(guild.name)),
        AuditChange::if_changed("icon_hash", json!(before.icon), json!(guild.icon)),
        AuditChange::if_changed(
            "description",
            json!(before.description),
            json!(guild.description),
        ),
        AuditChange::if_changed(
            "region",
            json!(before.voice_region),
            json!(guild.voice_region),
        ),
        AuditChange::if_changed(
            "owner_id",
            json!(before.owner_id.to_string()),
            json!(guild.owner_id.to_string()),
        ),
        AuditChange::if_changed(
            "verification_level",
            json!(before.verification_level),
            json!(guild.verification_level),
        ),
        AuditChange::if_changed(
            "afk_timeout",
            json!(before.afk_timeout),
            json!(guild.afk_timeout),
        ),
        AuditChange::if_changed(
            "afk_channel_id",
            json!(before.afk_channel_id.map(|id| id.to_string())),
            json!(guild.afk_channel_id.map(|id| id.to_string())),
        ),
        AuditChange::if_changed(
            "system_channel_id",
            json!(before.system_channel_id.map(|id| id.to_string())),
            json!(guild.system_channel_id.map(|id| id.to_string())),
        ),
        AuditChange::if_changed(
            "system_channel_flags",
            json!(before.system_channel_flags),
            json!(guild.system_channel_flags),
        ),
        AuditChange::if_changed(
            "default_message_notifications",
            json!(before.default_message_notifications),
            json!(guild.default_message_notifications),
        ),
        AuditChange::if_changed(
            "prune_admin_only",
            json!(before.prune_admin_only),
            json!(guild.prune_admin_only),
        ),
        AuditChange::if_changed("banner_hash", json!(before.banner), json!(guild.banner)),
        AuditChange::if_changed("splash_hash", json!(before.splash), json!(guild.splash)),
    ]
    .into_iter()
    .flatten()
    .collect();

    let mut writes = WriteSet::new();
    if community_requested {
        writes.mutate(Mutation::SetCommunity {
            guild_id,
            channels: guild.community,
        });
    }
    if before.audit_policy != guild.audit_policy {
        writes.mutate(Mutation::SetAuditPolicy {
            guild_id,
            policy: guild.audit_policy,
        });
    }
    writes
        .mutate(Mutation::UpdateGuild(guild.clone()))
        .emit(DomainEvent::GuildUpdate(guild.clone()));
    if !changes.is_empty() {
        writes.audit(audit.entry(
            ports,
            guild_id,
            AuditAction::GuildUpdate,
            Some(guild_id),
            changes,
        )?);
    }
    ports.unit_of_work.commit(writes).await?;
    Ok(guild)
}

async fn store_optional(
    ports: &Ports,
    prefix: &str,
    image: Option<ImageUpload>,
) -> AppResult<Option<String>> {
    match image {
        Some(image) => Ok(Some(store_hashed(ports, prefix, image).await?)),
        None => Ok(None),
    }
}

async fn ensure_channel_kind(
    ports: &Ports,
    guild_id: Snowflake,
    channel_id: Snowflake,
    accepts: impl Fn(ChannelType) -> bool,
    field: &str,
) -> AppResult<()> {
    let ok = ports
        .channels
        .find(channel_id)
        .await?
        .is_some_and(|c| c.guild_id == Some(guild_id) && accepts(c.kind));
    if !ok {
        return Err(AppError::invalid(
            field,
            "CHANNEL_INVALID",
            "Invalid channel.",
        ));
    }
    Ok(())
}
