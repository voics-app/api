pub mod builtins;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Resource};
use crate::events::DomainEvent;
use crate::support::guild_context::GuildContext;
use crate::support::images::ImageUpload;
use crate::support::stickers::{self, StickerFormat};
use crate::write_set::{Mutation, WriteSet};
use domain::permissions::Permissions;
use domain::sticker::{Sticker, MAX_STICKERS_PER_GUILD};
use domain::Snowflake;

fn validate_metadata(name: &str, tags: &str) -> AppResult<()> {
    if !(2..=30).contains(&name.chars().count()) || tags.chars().count() > 200 {
        return Err(AppError::invalid(
            "name",
            "INVALID_STICKER",
            "Invalid sticker name or tags.",
        ));
    }
    Ok(())
}

async fn context(ports: &Ports, actor: &Actor, guild_id: Snowflake, manage: bool) -> AppResult<()> {
    let ctx = GuildContext::load(ports, guild_id, actor.user_id).await?;
    if manage
        && !ctx
            .permissions()
            .contains(Permissions::MANAGE_GUILD_EXPRESSIONS)
    {
        return Err(AppError::missing_permissions());
    }
    Ok(())
}

pub async fn list(ports: &Ports, actor: &Actor, guild_id: Snowflake) -> AppResult<Vec<Sticker>> {
    context(ports, actor, guild_id, false).await?;
    ports.stickers.list(guild_id).await
}

pub async fn get(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    id: Snowflake,
) -> AppResult<Sticker> {
    context(ports, actor, guild_id, false).await?;
    ports
        .stickers
        .find(guild_id, id)
        .await?
        .ok_or(AppError::NotFound(Resource::Sticker))
}

pub struct CreateStickerInput {
    pub name: String,
    pub tags: String,
    pub image: ImageUpload,
}

pub async fn create(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    input: CreateStickerInput,
) -> AppResult<Sticker> {
    let CreateStickerInput { name, tags, image } = input;
    context(ports, actor, guild_id, true).await?;
    validate_metadata(&name, &tags)?;
    if ports.stickers.list(guild_id).await?.len() >= MAX_STICKERS_PER_GUILD {
        return Err(AppError::BadRequest {
            code: 30039,
            message: "Maximum number of stickers reached".into(),
        });
    }
    let format = stickers::validate(&image)?;
    let id = ports.ids.next_id()?;
    let extension = match format {
        StickerFormat::Png | StickerFormat::Apng => "png",
        StickerFormat::Gif => "gif",
        StickerFormat::Lottie => "json",
    };
    ports
        .assets
        .put(
            &format!("stickers/{id}.{extension}"),
            image.bytes,
            &image.content_type,
        )
        .await?;
    let sticker = Sticker {
        id,
        guild_id,
        creator_id: Some(actor.user_id),
        name,
        tags,
        format_type: format as i16,
        available: true,
    };
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::InsertSticker(sticker.clone()))
        .emit(DomainEvent::StickersUpdate {
            guild_id,
            stickers: vec![],
        });
    ports.unit_of_work.commit(writes).await?;
    Ok(sticker)
}

pub struct UpdateStickerInput {
    pub name: Option<String>,
    pub tags: Option<String>,
    pub remove: bool,
}

pub async fn update(
    ports: &Ports,
    actor: &Actor,
    guild_id: Snowflake,
    id: Snowflake,
    input: UpdateStickerInput,
) -> AppResult<Sticker> {
    let UpdateStickerInput { name, tags, remove } = input;
    context(ports, actor, guild_id, true).await?;
    let mut sticker = ports
        .stickers
        .find(guild_id, id)
        .await?
        .ok_or(AppError::NotFound(Resource::Sticker))?;
    if let Some(name) = name {
        sticker.name = name;
    }
    if let Some(tags) = tags {
        sticker.tags = tags;
    }
    validate_metadata(&sticker.name, &sticker.tags)?;
    if remove {
        sticker.available = false;
    }
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpdateSticker(sticker.clone()))
        .emit(DomainEvent::StickersUpdate {
            guild_id,
            stickers: vec![],
        });
    ports.unit_of_work.commit(writes).await?;
    Ok(sticker)
}

/// Resolve a guild sticker for a specific author and destination on the messages service.
pub async fn resolve_for_message(
    ports: &Ports,
    channel_id: Snowflake,
    user_id: Snowflake,
    sticker_id: Snowflake,
) -> AppResult<Sticker> {
    let access =
        crate::use_cases::internal::channel_access::execute(ports, channel_id, user_id).await?;
    if !access
        .permissions
        .contains(Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES)
        || access.delivery_mode != domain::delivery_mode::DeliveryMode::Standard
    {
        return Err(AppError::missing_access());
    }
    if let Some(sticker) = builtins::find(sticker_id) {
        return Ok(sticker);
    }
    let guild_id = access.guild_id.ok_or_else(AppError::missing_access)?;
    ports
        .stickers
        .find(guild_id, sticker_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Sticker))
}
