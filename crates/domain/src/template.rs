//! Server templates: a shareable snapshot of a guild's settings, roles and channels (api#44).
//!
//! Ids inside a template are template-local (`@everyone` is role 0, other roles and all
//! channels count from 1), so a template can be instantiated into any number of guilds.

use std::collections::HashMap;

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{json, Value};

use crate::channel::{Channel, ChannelType};
use crate::guild::{Guild, Role};
use crate::permissions::{OverwriteKind, PermissionOverwrite, Permissions};
use crate::snowflake::Snowflake;

/// Codes of built-in templates start with this; generated codes are alphanumeric.
pub const BUILTIN_PREFIX: &str = "builtin-";
pub const MAX_ROLES: usize = 250;
pub const MAX_CHANNELS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateOverwrite {
    /// Template-local role id.
    pub role: u64,
    pub allow: Permissions,
    pub deny: Permissions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateRole {
    pub id: u64,
    pub name: String,
    pub permissions: Permissions,
    pub color: u32,
    pub hoist: bool,
    pub mentionable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateChannel {
    pub id: u64,
    pub kind: ChannelType,
    pub name: String,
    pub position: i32,
    pub topic: Option<String>,
    pub nsfw: bool,
    pub bitrate: Option<u32>,
    pub user_limit: Option<u32>,
    pub rate_limit_per_user: u32,
    pub parent_id: Option<u64>,
    pub overwrites: Vec<TemplateOverwrite>,
}

/// What a template creates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateSource {
    pub name: String,
    pub description: Option<String>,
    pub verification_level: u8,
    pub default_message_notifications: u8,
    pub explicit_content_filter: u8,
    pub afk_timeout: u32,
    pub afk_channel_id: Option<u64>,
    pub system_channel_id: Option<u64>,
    /// `@everyone` (id 0) first, then the rest by ascending position.
    pub roles: Vec<TemplateRole>,
    pub channels: Vec<TemplateChannel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildTemplate {
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub usage_count: u32,
    pub creator_id: Option<Snowflake>,
    pub source_guild_id: Option<Snowflake>,
    pub source: TemplateSource,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl GuildTemplate {
    pub fn is_builtin(&self) -> bool {
        is_builtin_code(&self.code)
    }
}

pub fn is_builtin_code(code: &str) -> bool {
    code.starts_with(BUILTIN_PREFIX)
}

// --- capture ---------------------------------------------------------------

/// Snapshots a guild. Managed (bot) roles, threads and member overwrites are left out.
pub fn capture(guild: &Guild, roles: &[Role], channels: &[Channel]) -> TemplateSource {
    let everyone_perms = roles
        .iter()
        .find(|r| r.is_everyone())
        .map_or(Permissions::DEFAULT_EVERYONE, |r| r.permissions);
    let mut others: Vec<&Role> = roles
        .iter()
        .filter(|r| !r.is_everyone() && !r.managed)
        .collect();
    others.sort_by_key(|r| (r.position, r.id));
    let mut role_ids: HashMap<Snowflake, u64> = HashMap::from([(guild.id, 0)]);
    let mut out_roles = vec![TemplateRole {
        id: 0,
        name: "@everyone".to_owned(),
        permissions: everyone_perms,
        color: 0,
        hoist: false,
        mentionable: false,
    }];
    for (index, role) in others.into_iter().enumerate() {
        let id = index as u64 + 1;
        role_ids.insert(role.id, id);
        out_roles.push(TemplateRole {
            id,
            name: role.name.clone(),
            permissions: role.permissions,
            color: role.color,
            hoist: role.hoist,
            mentionable: role.mentionable,
        });
    }

    let mut kept: Vec<&Channel> = channels
        .iter()
        .filter(|c| c.kind.is_creatable_in_guild())
        .collect();
    kept.sort_by_key(|c| c.id);
    let channel_ids: HashMap<Snowflake, u64> = kept
        .iter()
        .enumerate()
        .map(|(index, c)| (c.id, index as u64 + 1))
        .collect();
    let out_channels = kept
        .iter()
        .map(|c| TemplateChannel {
            id: channel_ids[&c.id],
            kind: c.kind,
            name: c.name.clone().unwrap_or_default(),
            position: c.position,
            topic: c.topic.clone(),
            nsfw: c.nsfw,
            bitrate: c.bitrate,
            user_limit: c.user_limit,
            rate_limit_per_user: c.rate_limit_per_user,
            parent_id: c.parent_id.and_then(|p| channel_ids.get(&p).copied()),
            overwrites: c
                .overwrites
                .iter()
                .filter(|o| o.kind == OverwriteKind::Role)
                .filter_map(|o| {
                    Some(TemplateOverwrite {
                        role: *role_ids.get(&o.id)?,
                        allow: o.allow,
                        deny: o.deny,
                    })
                })
                .collect(),
        })
        .collect();
    TemplateSource {
        name: guild.name.clone(),
        description: guild.description.clone(),
        verification_level: guild.verification_level,
        default_message_notifications: guild.default_message_notifications,
        explicit_content_filter: guild.explicit_content_filter,
        afk_timeout: guild.afk_timeout,
        afk_channel_id: guild
            .afk_channel_id
            .and_then(|id| channel_ids.get(&id).copied()),
        system_channel_id: guild
            .system_channel_id
            .and_then(|id| channel_ids.get(&id).copied()),
        roles: out_roles,
        channels: out_channels,
    }
}

// --- instantiate -----------------------------------------------------------

/// The rows a template creates for a new guild.
#[derive(Debug, Clone)]
pub struct Instantiated {
    pub guild: Guild,
    /// `@everyone` first.
    pub roles: Vec<Role>,
    pub channels: Vec<Channel>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct TemplateError(pub String);

fn bad(message: &str) -> TemplateError {
    TemplateError(message.to_owned())
}

/// Builds a guild named `name` from `source`; `next_id` mints Snowflakes.
pub fn instantiate(
    source: &TemplateSource,
    name: &str,
    owner_id: Snowflake,
    voice_region: Option<String>,
    now: DateTime<Utc>,
    mut next_id: impl FnMut() -> Snowflake,
) -> Result<Instantiated, TemplateError> {
    instantiate_with_ids(
        source,
        name,
        owner_id,
        voice_region,
        now,
        std::iter::from_fn(|| Some(next_id())),
    )
}

/// Uses a finite set of IDs reserved by a fallible application port before construction.
pub fn instantiate_with_ids(
    source: &TemplateSource,
    name: &str,
    owner_id: Snowflake,
    voice_region: Option<String>,
    now: DateTime<Utc>,
    mut ids: impl Iterator<Item = Snowflake>,
) -> Result<Instantiated, TemplateError> {
    let mut next_id = || {
        ids.next()
            .ok_or_else(|| bad("Insufficient allocated identifiers."))
    };
    if source.roles.len() > MAX_ROLES || source.channels.len() > MAX_CHANNELS {
        return Err(bad("The template is too large."));
    }
    let guild_id = next_id()?;
    let mut role_ids: HashMap<u64, Snowflake> = HashMap::from([(0, guild_id)]);
    let everyone_permissions = source
        .roles
        .iter()
        .find(|r| r.id == 0)
        .map_or(Permissions::DEFAULT_EVERYONE, |r| r.permissions);
    let mut roles = vec![Role::everyone(
        guild_id,
        everyone_permissions & Permissions::ALL,
    )];
    let mut others: Vec<&TemplateRole> = source.roles.iter().filter(|r| r.id != 0).collect();
    others.sort_by_key(|r| r.id);
    for (index, role) in others.into_iter().enumerate() {
        let id = next_id()?;
        if role_ids.insert(role.id, id).is_some() {
            return Err(bad("Duplicate role id in the template."));
        }
        roles.push(Role {
            id,
            guild_id,
            name: role.name.clone(),
            color: role.color,
            hoist: role.hoist,
            icon: None,
            unicode_emoji: None,
            position: index as i32 + 1,
            permissions: role.permissions & Permissions::ALL,
            managed: false,
            mentionable: role.mentionable,
            bot_id: None,
        });
    }

    let mut channel_ids: HashMap<u64, Snowflake> = HashMap::new();
    for channel in &source.channels {
        if channel_ids.insert(channel.id, next_id()?).is_some() {
            return Err(bad("Duplicate channel id in the template."));
        }
    }
    let mut channels = Vec::with_capacity(source.channels.len());
    for template in &source.channels {
        let mut channel = Channel::new(
            channel_ids[&template.id],
            template.kind,
            Some(guild_id),
            now,
        );
        channel.name = Some(template.name.clone());
        channel.position = template.position;
        channel.topic = template.topic.clone();
        channel.nsfw = template.nsfw;
        if template.kind.is_voice() {
            channel.bitrate = template.bitrate.or(channel.bitrate);
            channel.user_limit = template.user_limit.or(channel.user_limit);
        }
        channel.rate_limit_per_user = template.rate_limit_per_user;
        channel.parent_id = match template.parent_id {
            None => None,
            Some(parent) => {
                let target = source
                    .channels
                    .iter()
                    .find(|c| c.id == parent)
                    .filter(|c| c.kind == ChannelType::GuildCategory)
                    .ok_or_else(|| bad("A channel's parent is not a category of the template."))?;
                Some(channel_ids[&target.id])
            }
        };
        for overwrite in &template.overwrites {
            let id = *role_ids
                .get(&overwrite.role)
                .ok_or_else(|| bad("A permission overwrite targets an unknown role."))?;
            channel.overwrites.push(PermissionOverwrite {
                id,
                kind: OverwriteKind::Role,
                allow: overwrite.allow & Permissions::ALL,
                deny: overwrite.deny & Permissions::ALL,
            });
        }
        channels.push(channel);
    }
    let channel_ref = |local: Option<u64>| local.and_then(|id| channel_ids.get(&id).copied());
    let guild = Guild {
        id: guild_id,
        name: name.trim().to_owned(),
        icon: None,
        description: source.description.clone(),
        owner_id,
        voice_region,
        verification_level: source.verification_level,
        default_message_notifications: source.default_message_notifications,
        explicit_content_filter: source.explicit_content_filter,
        afk_channel_id: channel_ref(source.afk_channel_id),
        afk_timeout: source.afk_timeout,
        system_channel_id: channel_ref(source.system_channel_id),
        system_channel_flags: 0,
        banner: None,
        splash: None,
        prune_admin_only: false,
        created_at: now,
    };
    Ok(Instantiated {
        guild,
        roles,
        channels,
    })
}

// --- JSON ------------------------------------------------------------------

impl TemplateSource {
    /// The Discord-style `serialized_source_guild` object.
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "verification_level": self.verification_level,
            "default_message_notifications": self.default_message_notifications,
            "explicit_content_filter": self.explicit_content_filter,
            "afk_timeout": self.afk_timeout,
            "afk_channel_id": self.afk_channel_id,
            "system_channel_id": self.system_channel_id,
            "roles": self.roles.iter().map(|r| json!({
                "id": r.id,
                "name": r.name,
                "permissions": r.permissions.bits().to_string(),
                "color": r.color,
                "hoist": r.hoist,
                "mentionable": r.mentionable,
            })).collect::<Vec<_>>(),
            "channels": self.channels.iter().map(|c| {
                let mut value = json!({
                    "id": c.id,
                    "type": c.kind.code(),
                    "name": c.name,
                    "position": c.position,
                    "topic": c.topic,
                    "nsfw": c.nsfw,
                    "rate_limit_per_user": c.rate_limit_per_user,
                    "parent_id": c.parent_id,
                    "permission_overwrites": c.overwrites.iter().map(|o| json!({
                        "id": o.role,
                        "type": 0,
                        "allow": o.allow.bits().to_string(),
                        "deny": o.deny.bits().to_string(),
                    })).collect::<Vec<_>>(),
                });
                if let Some(bitrate) = c.bitrate {
                    value["bitrate"] = json!(bitrate);
                }
                if let Some(limit) = c.user_limit {
                    value["user_limit"] = json!(limit);
                }
                value
            }).collect::<Vec<_>>(),
        })
    }

    /// Parses what `to_json` wrote.
    pub fn from_json(value: &Value) -> Result<Self, TemplateError> {
        let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
        let number = |v: &Value, key: &str| v.get(key).and_then(Value::as_u64);
        let perms = |v: &Value, key: &str| {
            v.get(key)
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<u64>().ok())
                .map(Permissions::from_bits_truncate)
                .ok_or_else(|| bad("Malformed permissions in the template."))
        };
        let missing = || bad("Malformed template.");
        let roles = value
            .get("roles")
            .and_then(Value::as_array)
            .ok_or_else(missing)?
            .iter()
            .map(|r| {
                Ok(TemplateRole {
                    id: number(r, "id").ok_or_else(missing)?,
                    name: text(r, "name").ok_or_else(missing)?,
                    permissions: perms(r, "permissions")?,
                    color: number(r, "color").unwrap_or(0) as u32,
                    hoist: r.get("hoist").and_then(Value::as_bool).unwrap_or(false),
                    mentionable: r
                        .get("mentionable")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                })
            })
            .collect::<Result<Vec<_>, TemplateError>>()?;
        let channels = value
            .get("channels")
            .and_then(Value::as_array)
            .ok_or_else(missing)?
            .iter()
            .map(|c| {
                let overwrites = c
                    .get("permission_overwrites")
                    .and_then(Value::as_array)
                    .map_or(Ok(Vec::new()), |list| {
                        list.iter()
                            .map(|o| {
                                Ok(TemplateOverwrite {
                                    role: number(o, "id").ok_or_else(missing)?,
                                    allow: perms(o, "allow")?,
                                    deny: perms(o, "deny")?,
                                })
                            })
                            .collect::<Result<Vec<_>, TemplateError>>()
                    })?;
                Ok(TemplateChannel {
                    id: number(c, "id").ok_or_else(missing)?,
                    kind: number(c, "type")
                        .and_then(|code| u8::try_from(code).ok())
                        .and_then(ChannelType::from_code)
                        .filter(|k| k.is_creatable_in_guild())
                        .ok_or_else(missing)?,
                    name: text(c, "name").ok_or_else(missing)?,
                    position: number(c, "position").unwrap_or(0) as i32,
                    topic: text(c, "topic"),
                    nsfw: c.get("nsfw").and_then(Value::as_bool).unwrap_or(false),
                    bitrate: number(c, "bitrate").map(|v| v as u32),
                    user_limit: number(c, "user_limit").map(|v| v as u32),
                    rate_limit_per_user: number(c, "rate_limit_per_user").unwrap_or(0) as u32,
                    parent_id: number(c, "parent_id"),
                    overwrites,
                })
            })
            .collect::<Result<Vec<_>, TemplateError>>()?;
        Ok(Self {
            name: text(value, "name").ok_or_else(missing)?,
            description: text(value, "description"),
            verification_level: number(value, "verification_level").unwrap_or(0) as u8,
            default_message_notifications: number(value, "default_message_notifications")
                .unwrap_or(0) as u8,
            explicit_content_filter: number(value, "explicit_content_filter").unwrap_or(0) as u8,
            afk_timeout: number(value, "afk_timeout").unwrap_or(300) as u32,
            afk_channel_id: number(value, "afk_channel_id"),
            system_channel_id: number(value, "system_channel_id"),
            roles,
            channels,
        })
    }
}

// --- built-in templates ----------------------------------------------------

/// Accumulates the channels of a built-in template.
struct Layout {
    channels: Vec<TemplateChannel>,
}

impl Layout {
    fn add(
        &mut self,
        kind: ChannelType,
        parent: Option<u64>,
        name: &str,
        topic: Option<&str>,
    ) -> u64 {
        let id = self.channels.len() as u64 + 1;
        let position = self
            .channels
            .iter()
            .filter(|c| {
                c.parent_id == parent
                    && (c.kind == ChannelType::GuildCategory)
                        == (kind == ChannelType::GuildCategory)
            })
            .count() as i32;
        self.channels.push(TemplateChannel {
            id,
            kind,
            name: name.to_owned(),
            position,
            topic: topic.map(str::to_owned),
            nsfw: false,
            bitrate: kind.is_voice().then_some(64_000),
            user_limit: kind.is_voice().then_some(0),
            rate_limit_per_user: 0,
            parent_id: parent,
            overwrites: Vec::new(),
        });
        id
    }

    fn category(&mut self, name: &str) -> u64 {
        self.add(ChannelType::GuildCategory, None, name, None)
    }

    fn text(&mut self, parent: u64, name: &str, topic: &str) -> u64 {
        self.add(ChannelType::GuildText, Some(parent), name, Some(topic))
    }

    fn announcements(&mut self, parent: u64, name: &str, topic: &str) -> u64 {
        self.add(
            ChannelType::GuildAnnouncement,
            Some(parent),
            name,
            Some(topic),
        )
    }

    fn voice(&mut self, parent: u64, name: &str, limit: u32) -> u64 {
        let id = self.add(ChannelType::GuildVoice, Some(parent), name, None);
        self.channels[id as usize - 1].user_limit = Some(limit);
        id
    }

    fn stage(&mut self, parent: u64, name: &str) -> u64 {
        self.add(ChannelType::GuildStageVoice, Some(parent), name, None)
    }

    /// Only staff may post; everyone can read.
    fn read_only(&mut self, channel: u64, staff: Option<u64>) {
        let target = &mut self.channels[channel as usize - 1];
        target.overwrites.push(TemplateOverwrite {
            role: 0,
            allow: Permissions::NONE,
            deny: Permissions::SEND_MESSAGES | Permissions::ADD_REACTIONS,
        });
        if let Some(role) = staff {
            target.overwrites.push(TemplateOverwrite {
                role,
                allow: Permissions::SEND_MESSAGES | Permissions::ADD_REACTIONS,
                deny: Permissions::NONE,
            });
        }
    }

    /// Hidden from everyone but `role`.
    fn staff_only(&mut self, channel: u64, role: u64) {
        let target = &mut self.channels[channel as usize - 1];
        target.overwrites.push(TemplateOverwrite {
            role: 0,
            allow: Permissions::NONE,
            deny: Permissions::VIEW_CHANNEL,
        });
        target.overwrites.push(TemplateOverwrite {
            role,
            allow: Permissions::VIEW_CHANNEL,
            deny: Permissions::NONE,
        });
    }
}

fn role(id: u64, name: &str, color: u32, hoist: bool, permissions: Permissions) -> TemplateRole {
    TemplateRole {
        id,
        name: name.to_owned(),
        permissions,
        color,
        hoist,
        mentionable: false,
    }
}

fn moderator_permissions() -> Permissions {
    Permissions::DEFAULT_EVERYONE
        | Permissions::KICK_MEMBERS
        | Permissions::BAN_MEMBERS
        | Permissions::MANAGE_MESSAGES
        | Permissions::MANAGE_THREADS
        | Permissions::MUTE_MEMBERS
        | Permissions::DEAFEN_MEMBERS
        | Permissions::MOVE_MEMBERS
        | Permissions::VIEW_AUDIT_LOG
}

fn source(
    name: &str,
    extra_roles: Vec<TemplateRole>,
    layout: Layout,
    afk: Option<u64>,
    system: Option<u64>,
) -> TemplateSource {
    let mut roles = vec![role(
        0,
        "@everyone",
        0,
        false,
        Permissions::DEFAULT_EVERYONE,
    )];
    roles.extend(extra_roles);
    TemplateSource {
        name: name.to_owned(),
        description: None,
        verification_level: 0,
        default_message_notifications: 1,
        explicit_content_filter: 0,
        afk_timeout: 300,
        afk_channel_id: afk,
        system_channel_id: system,
        roles,
        channels: layout.channels,
    }
}

fn builtin_template(
    slug: &str,
    name: &str,
    description: &str,
    source: TemplateSource,
) -> GuildTemplate {
    let published = Utc
        .with_ymd_and_hms(2026, 11, 3, 0, 0, 0)
        .single()
        .expect("valid constant date");
    GuildTemplate {
        code: format!("{BUILTIN_PREFIX}{slug}"),
        name: name.to_owned(),
        description: Some(description.to_owned()),
        usage_count: 0,
        creator_id: None,
        source_guild_id: None,
        source,
        created_at: published,
        updated_at: published,
    }
}

/// The templates offered by the create-server dialog.
pub fn builtins() -> Vec<GuildTemplate> {
    vec![gaming(), friends(), study_group(), community(), creators()]
}

pub fn builtin(code: &str) -> Option<GuildTemplate> {
    builtins().into_iter().find(|t| t.code == code)
}

fn gaming() -> GuildTemplate {
    let mut l = Layout { channels: vec![] };
    let info = l.category("Info");
    let rules = l.text(info, "welcome-and-rules", "Read this first.");
    l.read_only(rules, Some(1));
    let news = l.announcements(info, "announcements", "News from the staff.");
    l.read_only(news, Some(1));
    let text = l.category("Text Channels");
    let general = l.text(text, "general", "Chat about anything.");
    l.text(text, "game-chat", "Talk about the games you play.");
    l.text(text, "clips-and-highlights", "Show off your best plays.");
    l.text(text, "looking-for-group", "Find people to play with.");
    let voice = l.category("Voice Channels");
    l.voice(voice, "Lobby", 0);
    l.voice(voice, "Squad 1", 5);
    l.voice(voice, "Squad 2", 5);
    let afk = l.voice(voice, "AFK", 0);
    builtin_template(
        "gaming",
        "Gaming",
        "Squad channels, clips and a place to find a group.",
        source(
            "My Gaming Server",
            vec![
                role(1, "Moderator", 0x3498DB, true, moderator_permissions()),
                role(2, "Gamer", 0x2ECC71, true, Permissions::DEFAULT_EVERYONE),
            ],
            l,
            Some(afk),
            Some(general),
        ),
    )
}

fn friends() -> GuildTemplate {
    let mut l = Layout { channels: vec![] };
    let text = l.category("Text Channels");
    let general = l.text(text, "general", "Hang out and chat.");
    l.text(text, "memes", "The best of the internet.");
    l.text(text, "photos", "Share moments with each other.");
    let voice = l.category("Voice Channels");
    l.voice(voice, "Hangout", 0);
    l.voice(voice, "Movie Night", 0);
    let afk = l.voice(voice, "AFK", 0);
    builtin_template(
        "friends",
        "Friends",
        "A cozy place for your group to chat and hang out.",
        source("My Friends Server", vec![], l, Some(afk), Some(general)),
    )
}

fn study_group() -> GuildTemplate {
    let mut l = Layout { channels: vec![] };
    let info = l.category("Welcome");
    let rules = l.text(info, "rules-and-info", "How this group works.");
    l.read_only(rules, Some(1));
    let study = l.category("Study");
    let general = l.text(study, "general", "Chat and check in.");
    l.text(study, "homework-help", "Ask questions, answer questions.");
    l.text(study, "resources", "Notes, links and cheat sheets.");
    l.text(study, "study-log", "Share what you got done today.");
    let voice = l.category("Study Rooms");
    l.voice(voice, "Study Room 1", 0);
    l.voice(voice, "Study Room 2", 0);
    l.voice(voice, "Quiet Room", 0);
    let afk = l.voice(voice, "Break Room", 0);
    builtin_template(
        "study-group",
        "Study Group",
        "Rooms for focus, homework help and shared resources.",
        source(
            "My Study Group",
            vec![role(1, "Tutor", 0xF1C40F, true, moderator_permissions())],
            l,
            Some(afk),
            Some(general),
        ),
    )
}

fn community() -> GuildTemplate {
    let mut l = Layout { channels: vec![] };
    let info = l.category("Information");
    let welcome = l.text(info, "welcome", "Say hello to new members.");
    l.read_only(welcome, Some(1));
    let rules = l.text(info, "rules", "The ground rules.");
    l.read_only(rules, Some(1));
    let news = l.announcements(info, "announcements", "Official updates.");
    l.read_only(news, Some(1));
    let community = l.category("Community");
    let general = l.text(community, "general", "Talk about anything.");
    l.text(community, "introductions", "Tell us about yourself.");
    l.text(community, "off-topic", "Everything else.");
    l.text(community, "suggestions", "Help shape the server.");
    let voice = l.category("Voice");
    l.voice(voice, "Lounge", 0);
    l.stage(voice, "Events");
    let afk = l.voice(voice, "AFK", 0);
    let staff = l.category("Staff");
    let mod_chat = l.text(staff, "mod-chat", "Private channel for moderators.");
    l.staff_only(mod_chat, 1);
    builtin_template(
        "community",
        "Community",
        "Rules, announcements, events and a staff area for a larger server.",
        source(
            "My Community",
            vec![role(
                1,
                "Moderator",
                0xE67E22,
                true,
                moderator_permissions(),
            )],
            l,
            Some(afk),
            Some(general),
        ),
    )
}

fn creators() -> GuildTemplate {
    let mut l = Layout { channels: vec![] };
    let info = l.category("Info");
    let news = l.announcements(info, "announcements", "New uploads, streams and news.");
    l.read_only(news, Some(1));
    let rules = l.text(info, "rules", "Keep it friendly.");
    l.read_only(rules, Some(1));
    let content = l.category("Content");
    l.text(content, "showcase", "Share your work.");
    l.text(content, "feedback", "Give and get constructive feedback.");
    l.text(content, "collabs", "Find people to make things with.");
    l.text(content, "resources", "Tools, tutorials and assets.");
    let chat = l.category("Chat");
    let general = l.text(chat, "general", "Talk with the community.");
    l.text(chat, "off-topic", "Everything else.");
    let voice = l.category("Voice");
    l.voice(voice, "Studio", 0);
    l.stage(voice, "Stream Hangout");
    let afk = l.voice(voice, "AFK", 0);
    builtin_template(
        "creators",
        "Creators",
        "Showcase, feedback and collaboration spaces for a creative community.",
        source(
            "My Creator Space",
            vec![
                role(1, "Moderator", 0x9B59B6, true, moderator_permissions()),
                role(
                    2,
                    "Supporter",
                    0xE91E63,
                    true,
                    Permissions::DEFAULT_EVERYONE,
                ),
            ],
            l,
            Some(afk),
            Some(general),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counter() -> impl FnMut() -> Snowflake {
        let mut next = 1000;
        move || {
            next += 1;
            Snowflake::new(next)
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 4, 12, 0, 0).unwrap()
    }

    #[test]
    fn five_builtins_with_unique_prefixed_codes() {
        let all = builtins();
        assert_eq!(all.len(), 5);
        let mut codes: Vec<_> = all.iter().map(|t| t.code.clone()).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), 5);
        assert!(all.iter().all(|t| t.is_builtin() && t.creator_id.is_none()));
        assert!(builtin("builtin-gaming").is_some());
        assert!(builtin("gaming").is_none());
    }

    #[test]
    fn every_builtin_instantiates_and_survives_json() {
        for template in builtins() {
            let parsed = TemplateSource::from_json(&template.source.to_json()).expect("parses");
            assert_eq!(parsed, template.source, "{}", template.code);
            let made = instantiate(
                &template.source,
                "Test",
                Snowflake::new(7),
                None,
                now(),
                counter(),
            )
            .expect("instantiates");
            assert_eq!(
                made.roles[0].id, made.guild.id,
                "@everyone shares the guild id"
            );
            assert_eq!(made.channels.len(), template.source.channels.len());
            assert!(made.guild.system_channel_id.is_some());
            assert!(made
                .channels
                .iter()
                .any(|c| c.kind == ChannelType::GuildText));
            // Every channel's parent is one of the new categories.
            for channel in &made.channels {
                if let Some(parent) = channel.parent_id {
                    assert!(made
                        .channels
                        .iter()
                        .any(|c| c.id == parent && c.kind == ChannelType::GuildCategory));
                }
            }
        }
    }

    #[test]
    fn community_has_staff_only_channel_and_moderator_role() {
        let template = builtin("builtin-community").unwrap();
        let made = instantiate(
            &template.source,
            "Hub",
            Snowflake::new(7),
            None,
            now(),
            counter(),
        )
        .unwrap();
        let moderator = made.roles.iter().find(|r| r.name == "Moderator").unwrap();
        let mod_chat = made
            .channels
            .iter()
            .find(|c| c.name.as_deref() == Some("mod-chat"))
            .unwrap();
        assert!(mod_chat
            .overwrites
            .iter()
            .any(|o| o.id == made.guild.id && o.deny.contains(Permissions::VIEW_CHANNEL)));
        assert!(mod_chat
            .overwrites
            .iter()
            .any(|o| o.id == moderator.id && o.allow.contains(Permissions::VIEW_CHANNEL)));
        assert_eq!(made.guild.name, "Hub");
    }

    #[test]
    fn instantiate_rejects_broken_references() {
        let mut source = builtin("builtin-friends").unwrap().source;
        source.channels[1].parent_id = Some(99);
        assert!(instantiate(&source, "X1", Snowflake::new(7), None, now(), counter()).is_err());

        let mut source = builtin("builtin-friends").unwrap().source;
        source.channels[1].overwrites.push(TemplateOverwrite {
            role: 42,
            allow: Permissions::NONE,
            deny: Permissions::NONE,
        });
        assert!(instantiate(&source, "X1", Snowflake::new(7), None, now(), counter()).is_err());

        let mut source = builtin("builtin-friends").unwrap().source;
        source.channels[1].parent_id = Some(2); // a text channel, not a category
        assert!(instantiate(&source, "X1", Snowflake::new(7), None, now(), counter()).is_err());
    }

    #[test]
    fn finite_id_reservation_must_cover_every_template_object() {
        let source = gaming().source;
        let required =
            1 + source.roles.iter().filter(|role| role.id != 0).count() + source.channels.len();
        let ids = (100..100 + required as u64 - 1).map(Snowflake::new);
        assert!(
            instantiate_with_ids(&source, "Template", Snowflake::new(7), None, now(), ids).is_err()
        );
        let ids = (100..100 + required as u64).map(Snowflake::new);
        assert!(
            instantiate_with_ids(&source, "Template", Snowflake::new(7), None, now(), ids).is_ok()
        );
    }

    #[test]
    fn capture_maps_ids_and_drops_members_managed_roles_and_threads() {
        let guild_id = Snowflake::new(500);
        let source = builtin("builtin-gaming").unwrap().source;
        let made = instantiate(&source, "Src", Snowflake::new(7), None, now(), {
            let mut next = 499;
            move || {
                next += 1;
                Snowflake::new(next)
            }
        })
        .unwrap();
        assert_eq!(made.guild.id, guild_id);
        let mut roles = made.roles.clone();
        let mut bot = roles[1].clone();
        bot.id = Snowflake::new(9_001);
        bot.managed = true;
        bot.name = "A Bot".into();
        roles.push(bot);
        let mut channels = made.channels.clone();
        channels[2].overwrites.push(PermissionOverwrite {
            id: Snowflake::new(31337),
            kind: OverwriteKind::Member,
            allow: Permissions::NONE,
            deny: Permissions::VIEW_CHANNEL,
        });
        let mut thread = Channel::new(
            Snowflake::new(9_002),
            ChannelType::PublicThread,
            Some(guild_id),
            now(),
        );
        thread.name = Some("a thread".into());
        channels.push(thread);

        let captured = capture(&made.guild, &roles, &channels);
        assert_eq!(captured.roles.len(), source.roles.len(), "bot role dropped");
        assert_eq!(
            captured.channels.len(),
            source.channels.len(),
            "thread dropped"
        );
        assert_eq!(
            captured.channels[2].overwrites,
            source.channels[2].overwrites
        );
        assert_eq!(captured.system_channel_id, source.system_channel_id);
        assert_eq!(captured.afk_channel_id, source.afk_channel_id);
        let again = instantiate(&captured, "Copy", Snowflake::new(8), None, now(), counter())
            .expect("capture is instantiable");
        assert_eq!(again.channels.len(), made.channels.len());
    }

    #[test]
    fn from_json_rejects_garbage() {
        assert!(TemplateSource::from_json(&json!({})).is_err());
        let mut value = builtin("builtin-friends").unwrap().source.to_json();
        value["roles"][0]["permissions"] = json!("not a number");
        assert!(TemplateSource::from_json(&value).is_err());
        let mut value = builtin("builtin-friends").unwrap().source.to_json();
        value["channels"][0]["type"] = json!(1);
        assert!(TemplateSource::from_json(&value).is_err());
    }
}
