use std::collections::HashSet;

use domain::guild::MemberProfile;
use domain::permissions::{OverwriteKind, Permissions};
use domain::Snowflake;
use serde_json::{json, Value};

use crate::context::{Actor, Ports};
use crate::events::DomainEvent;
use crate::support::guild_context::GuildContext;
use crate::support::views::require_user;
use crate::write_set::{Mutation, WriteSet};
use crate::{AppError, AppResult};

// Self-service roles may grant ordinary conversation permissions, never moderation.
pub const SAFE_ROLE_BITS: u64 = (1 << 6)
    | (1 << 9)
    | (1 << 10)
    | (1 << 11)
    | (1 << 12)
    | (1 << 14)
    | (1 << 15)
    | (1 << 16)
    | (1 << 18)
    | (1 << 20)
    | (1 << 21)
    | (1 << 25)
    | (1 << 26)
    | (1 << 31)
    | (1 << 32)
    | (1 << 35)
    | (1 << 36)
    | (1 << 37)
    | (1 << 38)
    | (1 << 39)
    | (1 << 42)
    | (1 << 45)
    | (1 << 46)
    | (1 << 49);

fn invalid() -> AppError {
    AppError::invalid(
        "onboarding",
        "BASE_TYPE_INVALID",
        "Provide up to five questions with one to ten valid options each.",
    )
}

pub async fn get(ports: &Ports, actor: &Actor, guild_id: Snowflake) -> AppResult<Value> {
    GuildContext::load(ports, guild_id, actor.user_id).await?;
    let (config, revision) = ports.guilds.onboarding(guild_id).await?;
    Ok(json!({"config": config, "revision": revision}))
}

pub async fn set(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    value: Value,
) -> AppResult<Value> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    ctx.require(Permissions::MANAGE_GUILD)?;
    let object = value.as_object().ok_or_else(invalid)?;
    let enabled = object
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(invalid)?;
    let questions = object
        .get("questions")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    if object.len() != 2 || questions.len() > 5 || (enabled && questions.is_empty()) {
        return Err(invalid());
    }
    let guild_channels = ports.channels.list_for_guild(guild_id).await?;
    for question in questions {
        let q = question.as_object().ok_or_else(invalid)?;
        let title = q.get("title").and_then(Value::as_str).ok_or_else(invalid)?;
        let options = q
            .get("options")
            .and_then(Value::as_array)
            .ok_or_else(invalid)?;
        if q.len() != 2
            || title.trim().is_empty()
            || title.chars().count() > 100
            || options.is_empty()
            || options.len() > 10
        {
            return Err(invalid());
        }
        for option in options {
            let o = option.as_object().ok_or_else(invalid)?;
            let label = o.get("label").and_then(Value::as_str).ok_or_else(invalid)?;
            if o.len() != 3 || label.trim().is_empty() || label.chars().count() > 100 {
                return Err(invalid());
            }
            if let Some(raw) = o.get("role_id").and_then(Value::as_str) {
                ctx.require(Permissions::MANAGE_ROLES)?;
                let id: Snowflake = raw.parse().map_err(|_| invalid())?;
                let role = ctx.role(id)?;
                ctx.require_manageable_role(role)?;
                if id == guild_id || role.managed || role.permissions.bits() & !SAFE_ROLE_BITS != 0
                {
                    return Err(invalid());
                }
                if guild_channels
                    .iter()
                    .flat_map(|channel| &channel.overwrites)
                    .any(|overwrite| {
                        overwrite.kind == OverwriteKind::Role
                            && overwrite.id == id
                            && overwrite.allow.bits() & !SAFE_ROLE_BITS != 0
                    })
                {
                    return Err(invalid());
                }
            } else if !o.get("role_id").is_some_and(Value::is_null) {
                return Err(invalid());
            }
            if let Some(raw) = o.get("channel_id").and_then(Value::as_str) {
                let id: Snowflake = raw.parse().map_err(|_| invalid())?;
                let channel = ports.channels.find(id).await?.ok_or_else(invalid)?;
                if channel.guild_id != Some(guild_id)
                    || !ctx
                        .channel_permissions(&channel, None)
                        .contains(Permissions::VIEW_CHANNEL)
                {
                    return Err(invalid());
                }
            } else if !o.get("channel_id").is_some_and(Value::is_null) {
                return Err(invalid());
            }
        }
    }
    let revision = ports.guilds.set_onboarding(guild_id, &value).await?;
    Ok(json!({"config": value, "revision": revision}))
}

pub async fn submit(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    revision: i64,
    choices: Vec<usize>,
) -> AppResult<Value> {
    actor.require_user()?;
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    if ctx.member.pending {
        return Err(AppError::missing_permissions());
    }
    let (config, current) = ports.guilds.onboarding(guild_id).await?;
    if current != revision {
        return Err(AppError::Conflict {
            code: 40001,
            message: "Onboarding changed; review it again.".into(),
        });
    }
    let questions = config["questions"].as_array().ok_or_else(invalid)?;
    if config["enabled"] != true || choices.len() != questions.len() {
        return Err(invalid());
    }
    let mut roles = HashSet::new();
    let mut channels = Vec::new();
    for (question, selected) in questions.iter().zip(choices) {
        let option = question["options"].get(selected).ok_or_else(invalid)?;
        if let Some(id) = option["role_id"].as_str() {
            roles.insert(id.parse::<Snowflake>().map_err(|_| invalid())?);
        }
        if let Some(id) = option["channel_id"].as_str() {
            channels.push(id.to_owned());
        }
    }
    let mut roles: Vec<_> = roles.into_iter().collect();
    roles.sort_unstable();
    let user = require_user(ports, actor.user_id).await?;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::ApplyOnboarding {
            guild_id,
            user_id: actor.user_id,
            revision,
            role_ids: roles,
        })
        .emit(DomainEvent::MemberUpdate(MemberProfile {
            member: ctx.member,
            user,
        }));
    ports.unit_of_work.commit(writes).await?;
    Ok(json!({"channel_ids": channels}))
}
