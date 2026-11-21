use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::channel_access::ChannelAccess;
use crate::support::guild_context::GuildContext;
use crate::write_set::{Mutation, WriteSet};
use domain::audit::{AuditAction, AuditChange};
use domain::channel::ChannelType;
use domain::permissions::Permissions;
use domain::soundboard::{
    valid_audio, valid_name, valid_volume, SoundboardMetadata, SoundboardSound,
};
use domain::Snowflake;
use serde_json::json;

fn editable(ctx: &GuildContext, sound: &SoundboardSound) -> AppResult<()> {
    let p = ctx.permissions();
    if p.contains(Permissions::MANAGE_GUILD_EXPRESSIONS)
        || (sound.creator_id == Some(ctx.member.user_id)
            && p.contains(Permissions::CREATE_GUILD_EXPRESSIONS))
    {
        Ok(())
    } else {
        Err(AppError::missing_permissions())
    }
}
fn validate(name: &str, volume: f32) -> AppResult<()> {
    if !valid_name(name) {
        return Err(AppError::invalid(
            "name",
            "INVALID",
            "Use 2–32 letters, digits, hyphens or underscores",
        ));
    }
    if !valid_volume(volume) {
        return Err(AppError::invalid(
            "volume",
            "INVALID",
            "Volume must be between 0 and 1",
        ));
    }
    Ok(())
}
pub async fn list(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
) -> AppResult<Vec<SoundboardSound>> {
    GuildContext::load(ports, guild_id, actor.user_id).await?;
    ports.soundboards.list_sounds(guild_id).await
}
pub async fn get(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    id: Snowflake,
) -> AppResult<SoundboardSound> {
    GuildContext::load(ports, guild_id, actor.user_id).await?;
    ports
        .soundboards
        .find_sound(guild_id, id)
        .await?
        .ok_or(AppError::NotFound(Resource::SoundboardSound))
}
pub async fn create(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    name: String,
    volume: f32,
    audio: Vec<u8>,
    audit: AuditContext<'_>,
) -> AppResult<SoundboardSound> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    if !ctx
        .permissions()
        .intersects(Permissions::CREATE_GUILD_EXPRESSIONS | Permissions::MANAGE_GUILD_EXPRESSIONS)
    {
        return Err(AppError::missing_permissions());
    }
    validate(&name, volume)?;
    if !valid_audio(&audio) {
        return Err(AppError::invalid(
            "sound",
            "INVALID",
            "Sound must be a canonical mono 48 kHz 16-bit PCM WAV, at most five seconds",
        ));
    }
    let sound = SoundboardSound {
        id: ports.ids.next_id()?,
        guild_id,
        creator_id: Some(actor.user_id),
        name,
        volume,
        audio,
    };
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::InsertSoundboardSound(sound.clone()))
        .emit(DomainEvent::SoundboardCreate {
            sound: SoundboardMetadata::from(&sound),
        })
        .audit(audit.entry(
            ports,
            guild_id,
            AuditAction::SoundboardCreate,
            Some(sound.id),
            vec![AuditChange::new("name", None, Some(json!(sound.name)))],
        )?);
    ports.unit_of_work.commit(writes).await?;
    Ok(sound)
}
pub async fn update(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    id: Snowflake,
    name: Option<String>,
    volume: Option<f32>,
    audit: AuditContext<'_>,
) -> AppResult<SoundboardSound> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    let mut sound = ports
        .soundboards
        .find_sound(guild_id, id)
        .await?
        .ok_or(AppError::NotFound(Resource::SoundboardSound))?;
    editable(&ctx, &sound)?;
    let before = SoundboardMetadata::from(&sound);
    if let Some(name) = name {
        sound.name = name;
    }
    if let Some(volume) = volume {
        sound.volume = volume;
    }
    validate(&sound.name, sound.volume)?;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpdateSoundboardSound(sound.clone()))
        .emit(DomainEvent::SoundboardUpdate {
            sound: SoundboardMetadata::from(&sound),
        })
        .audit(
            audit.entry(
                ports,
                guild_id,
                AuditAction::SoundboardUpdate,
                Some(sound.id),
                [
                    AuditChange::if_changed("name", json!(before.name), json!(sound.name)),
                    AuditChange::if_changed("volume", json!(before.volume), json!(sound.volume)),
                ]
                .into_iter()
                .flatten()
                .collect(),
            )?,
        );
    ports.unit_of_work.commit(writes).await?;
    Ok(sound)
}
pub async fn delete(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    id: Snowflake,
    audit: AuditContext<'_>,
) -> AppResult<()> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    let sound = ports
        .soundboards
        .find_sound(guild_id, id)
        .await?
        .ok_or(AppError::NotFound(Resource::SoundboardSound))?;
    editable(&ctx, &sound)?;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::DeleteSoundboardSound {
            guild_id,
            sound_id: id,
        })
        .emit(DomainEvent::SoundboardDelete {
            guild_id,
            sound_id: id,
        })
        .audit(audit.entry(
            ports,
            guild_id,
            AuditAction::SoundboardDelete,
            Some(id),
            vec![AuditChange::new("name", Some(json!(sound.name)), None)],
        )?);
    ports.unit_of_work.commit(writes).await
}

/// Authorizes a play request; actual RTP publication is independently gated by the SFU.
pub async fn authorize_send(
    ports: &Ports,
    actor: &Actor,
    channel_id: Snowflake,
    source_guild_id: Snowflake,
    sound_id: Snowflake,
) -> AppResult<()> {
    let access = ChannelAccess::visible(ports, channel_id, actor.user_id).await?;
    // Stage publication status is not yet authoritative across the gateway and SFU.
    if access.channel.kind != ChannelType::GuildVoice {
        return Err(AppError::missing_access());
    }
    access.require(Permissions::CONNECT | Permissions::SPEAK | Permissions::USE_SOUNDBOARD)?;
    let guild_id = access
        .channel
        .guild_id
        .ok_or_else(AppError::missing_access)?;
    if source_guild_id != guild_id {
        access.require(Permissions::USE_EXTERNAL_SOUNDS)?;
        GuildContext::load(ports, source_guild_id, actor.user_id).await?;
    }
    ports
        .soundboards
        .find_sound(source_guild_id, sound_id)
        .await?
        .ok_or(AppError::NotFound(Resource::SoundboardSound))?;
    // The publication snapshot is also the authoritative voice-membership check. Keeping both
    // checks in one gateway read prevents a leave/rejoin (or another socket for the same user)
    // from changing the owning session between a membership lookup and the capability lookup.
    if !ports
        .presence
        .can_publish_soundboard(guild_id, channel_id, actor.user_id)
        .await?
    {
        return Err(AppError::missing_permissions());
    }
    Ok(())
}
