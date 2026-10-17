use domain::channel::{Channel, ChannelType};
use domain::guild::{Guild, GuildSnapshot, Member, MemberProfile, Role};
use domain::permissions::Permissions;
use domain::validation;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::events::DomainEvent;
use crate::support::images::{store_hashed, ImageUpload};
use crate::support::views::require_user;
use crate::write_set::{Mutation, WriteSet};

/// A user may own or belong to at most this many guilds.
pub const MAX_GUILDS_PER_USER: usize = 200;

#[derive(Debug, Clone, Default)]
pub struct CreateGuildInput {
    pub name: String,
    pub icon: Option<ImageUpload>,
    pub voice_region: Option<String>,
}

pub fn ensure_known_region(ports: &Ports, region: &str) -> AppResult<()> {
    if ports.settings.voice_regions.iter().any(|r| r.id == region) {
        Ok(())
    } else {
        Err(AppError::invalid("voice_region", "BASE_TYPE_CHOICES", "Unknown voice region."))
    }
}

/// Creates a guild with @everyone, two categories and a text and voice channel.
pub async fn execute(ports: &Ports, actor: &Actor, input: CreateGuildInput) -> AppResult<GuildSnapshot> {
    actor.require_user()?;
    validation::guild_name(&input.name)?;
    if let Some(region) = &input.voice_region {
        ensure_known_region(ports, region)?;
    }
    if ports.guilds.list_for_user(actor.user_id).await?.len() >= MAX_GUILDS_PER_USER {
        return Err(AppError::BadRequest { code: 30001, message: "Maximum number of guilds reached (200)".into() });
    }
    let owner = require_user(ports, actor.user_id).await?;
    let now = ports.clock.now();
    let guild_id = ports.ids.next_id();
    let icon = match input.icon {
        Some(image) => Some(store_hashed(ports, &format!("icons/{guild_id}"), image).await?),
        None => None,
    };

    let channel = |kind: ChannelType, name: &str, position: i32, parent: Option<&Channel>| {
        let mut c = Channel::new(ports.ids.next_id(), kind, Some(guild_id), now);
        c.name = Some(name.to_owned());
        c.position = position;
        c.parent_id = parent.map(|p| p.id);
        c
    };
    let text_category = channel(ChannelType::GuildCategory, "Text Channels", 0, None);
    let voice_category = channel(ChannelType::GuildCategory, "Voice Channels", 1, None);
    let general = channel(ChannelType::GuildText, "general", 0, Some(&text_category));
    let lounge = channel(ChannelType::GuildVoice, "General", 0, Some(&voice_category));

    let guild = Guild {
        id: guild_id,
        name: input.name.trim().to_owned(),
        icon,
        description: None,
        owner_id: actor.user_id,
        voice_region: input.voice_region,
        verification_level: 0,
        default_message_notifications: 1,
        explicit_content_filter: 0,
        afk_channel_id: None,
        afk_timeout: 300,
        system_channel_id: Some(general.id),
        created_at: now,
    };
    let everyone = Role::everyone(guild_id, Permissions::DEFAULT_EVERYONE);
    let member = Member::new(guild_id, actor.user_id, now);
    let channels = vec![text_category, voice_category, general, lounge];

    let snapshot = GuildSnapshot {
        guild: guild.clone(),
        roles: vec![everyone.clone()],
        channels: channels.clone(),
        members: vec![MemberProfile { member: member.clone(), user: owner }],
        emojis: vec![],
        member_count: 1,
    };
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::InsertGuild(guild)).mutate(Mutation::InsertRole(everyone));
    for c in channels {
        writes.mutate(Mutation::InsertChannel(c));
    }
    writes.mutate(Mutation::UpsertMember(member)).emit(DomainEvent::GuildCreate {
        snapshot: Box::new(snapshot.clone()),
        recipients: vec![actor.user_id],
    });
    ports.unit_of_work.commit(writes).await?;
    Ok(snapshot)
}
