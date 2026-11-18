//! Maps domain events onto P6 envelopes: topic, partition key and routing.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use application::events::{DomainEvent, PrivateChannelView};
use contracts::envelope::{
    Envelope, TOPIC_AUDIT, TOPIC_GUILD, TOPIC_INTERACTION, TOPIC_MESSAGE, TOPIC_USER,
};
use contracts::objects::{
    interaction_event, AuditLogEntryDto, ChannelDto, DeviceDto, EmojiDto, GuildDto, GuildFullDto,
    GuildNotificationSettingsDto, MemberDto, RelationshipDto, RoleDto, UserDto, UserSettingsDto,
};
use contracts::Id;
use domain::relationship::Relationship;
use domain::Snowflake;

/// One envelope ready for the outbox, with the topic and partition key it goes to.
#[derive(Debug, Clone, PartialEq)]
pub struct Routed {
    pub topic: &'static str,
    pub key: String,
    pub envelope: Envelope,
}

/// Maps one domain event onto zero or more envelopes (P6).
pub fn route(event: &DomainEvent, default_region: &str, occurred_at: DateTime<Utc>) -> Vec<Routed> {
    let mut out = Vec::with_capacity(1);
    let mut emit = |topic: &'static str,
                    key: Snowflake,
                    kind: &str,
                    guild: Option<Snowflake>,
                    channel: Option<Snowflake>,
                    targets: Option<Vec<Snowflake>>,
                    data: Value| {
        out.push(Routed {
            topic,
            key: key.to_string(),
            envelope: Envelope::new(
                Uuid::now_v7(),
                kind,
                occurred_at,
                guild.map(Id),
                channel.map(Id),
                targets.map(|ids| ids.into_iter().map(Id).collect()),
                data,
            ),
        });
    };

    match event {
        DomainEvent::GuildCreate {
            snapshot,
            recipients,
        } => emit(
            TOPIC_GUILD,
            snapshot.guild.id,
            "GUILD_CREATE",
            Some(snapshot.guild.id),
            None,
            Some(recipients.clone()),
            to_value(GuildFullDto::new(snapshot, default_region)),
        ),
        DomainEvent::GuildUpdate(guild) => emit(
            TOPIC_GUILD,
            guild.id,
            "GUILD_UPDATE",
            Some(guild.id),
            None,
            None,
            to_value(GuildDto::new(guild, &[], &[], default_region)),
        ),
        DomainEvent::GuildDelete {
            guild_id,
            recipients,
        } => emit(
            TOPIC_GUILD,
            *guild_id,
            "GUILD_DELETE",
            Some(*guild_id),
            None,
            recipients.clone(),
            json!({ "id": guild_id.to_string() }),
        ),
        DomainEvent::ChannelCreate(channel) => guild_channel(&mut emit, "CHANNEL_CREATE", channel),
        DomainEvent::ChannelUpdate(channel) => guild_channel(&mut emit, "CHANNEL_UPDATE", channel),
        DomainEvent::ChannelDelete(channel) => guild_channel(&mut emit, "CHANNEL_DELETE", channel),
        DomainEvent::ThreadCreate(channel) => guild_channel(&mut emit, "THREAD_CREATE", channel),
        DomainEvent::ThreadUpdate(channel) => guild_channel(&mut emit, "THREAD_UPDATE", channel),
        DomainEvent::ThreadDelete(channel) => guild_channel(&mut emit, "THREAD_DELETE", channel),
        DomainEvent::RoleCreate(role) => emit(
            TOPIC_GUILD,
            role.guild_id,
            "GUILD_ROLE_CREATE",
            Some(role.guild_id),
            None,
            None,
            json!({ "guild_id": role.guild_id.to_string(), "role": RoleDto::from(role) }),
        ),
        DomainEvent::RoleUpdate(role) => emit(
            TOPIC_GUILD,
            role.guild_id,
            "GUILD_ROLE_UPDATE",
            Some(role.guild_id),
            None,
            None,
            json!({ "guild_id": role.guild_id.to_string(), "role": RoleDto::from(role) }),
        ),
        DomainEvent::RoleDelete { guild_id, role_id } => emit(
            TOPIC_GUILD,
            *guild_id,
            "GUILD_ROLE_DELETE",
            Some(*guild_id),
            None,
            None,
            json!({ "guild_id": guild_id.to_string(), "role_id": role_id.to_string() }),
        ),
        DomainEvent::MemberAdd(profile) => {
            let guild_id = profile.member.guild_id;
            emit(
                TOPIC_GUILD,
                guild_id,
                "GUILD_MEMBER_ADD",
                Some(guild_id),
                None,
                None,
                member_payload(profile),
            )
        }
        DomainEvent::MemberUpdate(profile) => {
            let guild_id = profile.member.guild_id;
            emit(
                TOPIC_GUILD,
                guild_id,
                "GUILD_MEMBER_UPDATE",
                Some(guild_id),
                None,
                None,
                member_payload(profile),
            )
        }
        DomainEvent::MemberRemove { guild_id, user } => emit(
            TOPIC_GUILD,
            *guild_id,
            "GUILD_MEMBER_REMOVE",
            Some(*guild_id),
            None,
            None,
            json!({ "guild_id": guild_id.to_string(), "user": UserDto::public(user) }),
        ),
        DomainEvent::VoiceMemberMove {
            guild_id,
            user_id,
            channel_id,
        } => emit(
            TOPIC_GUILD,
            *guild_id,
            "VOICE_MEMBER_MOVE",
            Some(*guild_id),
            None,
            None,
            json!({
                "guild_id": guild_id.to_string(),
                "user_id": user_id.to_string(),
                "channel_id": channel_id.map(|id| id.to_string()),
            }),
        ),
        DomainEvent::BanMessagePurge {
            guild_id,
            user_id,
            channel_ids,
            since_ms,
            delete_message_seconds,
        } => emit(
            TOPIC_GUILD,
            *guild_id,
            "BAN_MESSAGE_PURGE",
            Some(*guild_id),
            None,
            None,
            json!({
                "guild_id": guild_id.to_string(),
                "user_id": user_id.to_string(),
                "channel_ids": channel_ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "since_ms": since_ms,
                "delete_message_seconds": delete_message_seconds,
            }),
        ),
        DomainEvent::BanAdd { guild_id, user } => emit(
            TOPIC_GUILD,
            *guild_id,
            "GUILD_BAN_ADD",
            Some(*guild_id),
            None,
            None,
            json!({ "guild_id": guild_id.to_string(), "user": UserDto::public(user) }),
        ),
        DomainEvent::BanRemove { guild_id, user } => emit(
            TOPIC_GUILD,
            *guild_id,
            "GUILD_BAN_REMOVE",
            Some(*guild_id),
            None,
            None,
            json!({ "guild_id": guild_id.to_string(), "user": UserDto::public(user) }),
        ),
        DomainEvent::EmojisUpdate { guild_id, emojis } => {
            let dtos: Vec<EmojiDto> = emojis.iter().map(EmojiDto::from).collect();
            emit(
                TOPIC_GUILD,
                *guild_id,
                "GUILD_EMOJIS_UPDATE",
                Some(*guild_id),
                None,
                None,
                json!({ "guild_id": guild_id.to_string(), "emojis": dtos }),
            )
        }
        DomainEvent::InviteCreate { invite, inviter } => emit(
            TOPIC_GUILD,
            invite.guild_id,
            "INVITE_CREATE",
            Some(invite.guild_id),
            Some(invite.channel_id),
            None,
            json!({
                "code": invite.code,
                "guild_id": invite.guild_id.to_string(),
                "channel_id": invite.channel_id.to_string(),
                "inviter": inviter.as_ref().map(UserDto::public),
                "max_age": invite.max_age,
                "max_uses": invite.max_uses,
                "uses": invite.uses,
                "temporary": invite.temporary,
                "created_at": invite.created_at.to_rfc3339(),
            }),
        ),
        DomainEvent::InviteDelete {
            guild_id,
            channel_id,
            code,
        } => emit(
            TOPIC_GUILD,
            *guild_id,
            "INVITE_DELETE",
            Some(*guild_id),
            Some(*channel_id),
            None,
            json!({ "code": code, "guild_id": guild_id.to_string(), "channel_id": channel_id.to_string() }),
        ),
        DomainEvent::UserUpdate(user) => emit(
            TOPIC_USER,
            user.id,
            "USER_UPDATE",
            None,
            None,
            Some(vec![user.id]),
            to_value(UserDto::private(user, "en-US")),
        ),
        DomainEvent::UserSettingsUpdate(settings) => emit(
            TOPIC_USER,
            settings.user_id,
            "USER_SETTINGS_UPDATE",
            None,
            None,
            Some(vec![settings.user_id]),
            to_value(UserSettingsDto::from(settings)),
        ),
        DomainEvent::UserGuildSettingsUpdate(settings) => emit(
            TOPIC_USER,
            settings.user_id,
            "USER_GUILD_SETTINGS_UPDATE",
            None,
            None,
            Some(vec![settings.user_id]),
            to_value(GuildNotificationSettingsDto::from(settings)),
        ),
        DomainEvent::UserNoteUpdate {
            owner_id,
            target_id,
            note,
        } => emit(
            TOPIC_USER,
            *owner_id,
            "USER_NOTE_UPDATE",
            None,
            None,
            Some(vec![*owner_id]),
            json!({ "id": target_id.to_string(), "note": note }),
        ),
        DomainEvent::RelationshipAdd {
            user_id,
            kind,
            target,
            since,
        } => {
            let relationship = Relationship {
                user_id: *user_id,
                target_id: target.id,
                kind: *kind,
                since: *since,
            };
            emit(
                TOPIC_USER,
                *user_id,
                "RELATIONSHIP_ADD",
                None,
                None,
                Some(vec![*user_id]),
                to_value(RelationshipDto::new(&relationship, target)),
            )
        }
        DomainEvent::RelationshipRemove {
            user_id,
            target_id,
            kind,
        } => emit(
            TOPIC_USER,
            *user_id,
            "RELATIONSHIP_REMOVE",
            None,
            None,
            Some(vec![*user_id]),
            json!({ "id": target_id.to_string(), "type": kind.code() }),
        ),
        DomainEvent::PrivateChannelCreate(view) => {
            private_channel(&mut emit, "CHANNEL_CREATE", view)
        }
        DomainEvent::PrivateChannelCreateFor { view, user_id } => emit(
            TOPIC_USER,
            *user_id,
            "CHANNEL_CREATE",
            None,
            Some(view.channel.id),
            Some(vec![*user_id]),
            to_value(ChannelDto::private(
                &view.channel,
                &view.recipients,
                view.delivery_mode,
                Some(*user_id),
            )),
        ),
        DomainEvent::PrivateChannelUpdate(view) => {
            private_channel(&mut emit, "CHANNEL_UPDATE", view)
        }
        DomainEvent::PrivateChannelDelete { channel, user_id } => emit(
            TOPIC_USER,
            *user_id,
            "CHANNEL_DELETE",
            None,
            Some(channel.id),
            Some(vec![*user_id]),
            to_value(ChannelDto::private(
                channel,
                &[],
                channel.delivery_mode,
                Some(*user_id),
            )),
        ),
        DomainEvent::RecipientAdd {
            channel_id,
            user,
            notify,
        } => emit(
            TOPIC_USER,
            notify.first().copied().unwrap_or(*channel_id),
            "CHANNEL_RECIPIENT_ADD",
            None,
            Some(*channel_id),
            Some(notify.clone()),
            json!({ "channel_id": channel_id.to_string(), "user": UserDto::public(user) }),
        ),
        DomainEvent::RecipientRemove {
            channel_id,
            user,
            notify,
        } => emit(
            TOPIC_USER,
            notify.first().copied().unwrap_or(*channel_id),
            "CHANNEL_RECIPIENT_REMOVE",
            None,
            Some(*channel_id),
            Some(notify.clone()),
            json!({ "channel_id": channel_id.to_string(), "user": UserDto::public(user) }),
        ),
        DomainEvent::SessionsRevoke {
            user_id,
            session_ids,
        } => emit(
            TOPIC_USER,
            *user_id,
            "SESSION_REVOKE",
            None,
            None,
            Some(vec![*user_id]),
            json!({ "session_ids": session_ids.iter().map(ToString::to_string).collect::<Vec<_>>() }),
        ),
        DomainEvent::UserDevicesUpdate {
            user_id,
            devices,
            target_user_ids,
        } => {
            let devices: Vec<DeviceDto> = devices.iter().map(DeviceDto::from).collect();
            emit(
                TOPIC_USER,
                *user_id,
                "USER_DEVICES_UPDATE",
                None,
                None,
                Some(target_user_ids.clone()),
                json!({ "user_id": user_id.to_string(), "devices": devices }),
            )
        }
        DomainEvent::TypingStart {
            channel_id,
            guild_id,
            recipient_ids,
            user_id,
            timestamp,
            member,
        } => {
            let targets = (!recipient_ids.is_empty()).then(|| recipient_ids.clone());
            emit(
                TOPIC_MESSAGE,
                *channel_id,
                "TYPING_START",
                *guild_id,
                Some(*channel_id),
                targets,
                json!({
                    "channel_id": channel_id.to_string(),
                    "guild_id": guild_id.map(|id| id.to_string()),
                    "user_id": user_id.to_string(),
                    "timestamp": timestamp.timestamp(),
                    "member": member.as_deref().map(MemberDto::from),
                }),
            )
        }
        DomainEvent::InteractionCreate {
            application_id,
            bot_user_id,
            interaction,
            token,
            app_permissions,
            user,
            member,
        } => emit(
            TOPIC_INTERACTION,
            *application_id,
            "INTERACTION_CREATE",
            interaction.guild_id,
            Some(interaction.channel_id),
            Some(vec![*bot_user_id]),
            interaction_event(
                interaction,
                token,
                *app_permissions,
                user,
                member.as_deref(),
            ),
        ),
        DomainEvent::InteractionUserNotice {
            application_id,
            user_id,
            event_type,
            payload,
        } => emit(
            TOPIC_INTERACTION,
            *application_id,
            event_type,
            None,
            None,
            Some(vec![*user_id]),
            payload.clone(),
        ),
        DomainEvent::AuditLogEntryCreate(entry) => emit(
            TOPIC_AUDIT,
            entry.guild_id,
            "AUDIT_LOG_ENTRY_CREATE",
            Some(entry.guild_id),
            entry.target_id,
            None,
            to_value(AuditLogEntryDto::from(entry)),
        ),
    }
    out
}

fn to_value<T: Serialize>(value: T) -> Value {
    serde_json::to_value(value).expect("DTOs serialize to JSON")
}

fn member_payload(profile: &domain::guild::MemberProfile) -> Value {
    let mut payload = to_value(MemberDto::from(profile));
    if let Some(fields) = payload.as_object_mut() {
        fields.insert(
            "guild_id".into(),
            json!(profile.member.guild_id.to_string()),
        );
    }
    payload
}

fn guild_channel(
    emit: &mut impl FnMut(
        &'static str,
        Snowflake,
        &str,
        Option<Snowflake>,
        Option<Snowflake>,
        Option<Vec<Snowflake>>,
        Value,
    ),
    kind: &str,
    channel: &domain::channel::Channel,
) {
    let guild_id = channel.guild_id;
    emit(
        TOPIC_GUILD,
        guild_id.unwrap_or(channel.id),
        kind,
        guild_id,
        Some(channel.id),
        None,
        to_value(ChannelDto::guild(channel)),
    );
}

fn private_channel(
    emit: &mut impl FnMut(
        &'static str,
        Snowflake,
        &str,
        Option<Snowflake>,
        Option<Snowflake>,
        Option<Vec<Snowflake>>,
        Value,
    ),
    kind: &str,
    view: &PrivateChannelView,
) {
    for recipient in &view.recipients {
        emit(
            TOPIC_USER,
            recipient.id,
            kind,
            None,
            Some(view.channel.id),
            Some(vec![recipient.id]),
            to_value(ChannelDto::private(
                &view.channel,
                &view.recipients,
                view.delivery_mode,
                Some(recipient.id),
            )),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use domain::channel::{Channel, ChannelType};
    use domain::guild::{Guild, Member, MemberProfile};
    use domain::user::User;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 18, 12, 0, 0).unwrap()
    }

    fn user(id: u64) -> User {
        User {
            id: Snowflake::new(id),
            username: format!("user{id}"),
            global_name: None,
            avatar: None,
            bio: None,
            banner: None,
            pronouns: None,
            theme_colors: None,
            accent_color: None,
            bot: false,
            system: false,
            email: None,
            verified: true,
            mfa_enabled: false,
            flags: 0,
            platform_admin: false,
            suspension: None,
            created_at: at(),
        }
    }

    #[test]
    fn guild_events_fan_out_on_the_guild_topic_keyed_by_guild() {
        let channel = Channel::new(
            Snowflake::new(5),
            ChannelType::GuildText,
            Some(Snowflake::new(1)),
            at(),
        );
        let routed = route(&DomainEvent::ChannelCreate(channel), "nl-home", at());
        assert_eq!(routed.len(), 1);
        assert_eq!(routed[0].topic, TOPIC_GUILD);
        assert_eq!(routed[0].key, "1");
        assert_eq!(routed[0].envelope.kind, "CHANNEL_CREATE");
        assert_eq!(routed[0].envelope.guild_id, Some(Id(Snowflake::new(1))));
        assert_eq!(routed[0].envelope.target_user_ids, None);
    }

    #[test]
    fn private_channel_creation_sends_one_envelope_per_recipient() {
        let channel = Channel::new(Snowflake::new(9), ChannelType::GroupDm, None, at());
        let view = PrivateChannelView {
            channel,
            recipients: vec![user(10), user(11)],
            delivery_mode: domain::delivery_mode::DeliveryMode::Standard,
        };
        let routed = route(&DomainEvent::PrivateChannelCreate(view), "nl-home", at());
        assert_eq!(routed.len(), 2);
        assert_eq!(routed[0].key, "10");
        assert_eq!(
            routed[0].envelope.target_user_ids,
            Some(vec![Id(Snowflake::new(10))])
        );
        assert_eq!(routed[1].key, "11");
    }

    #[test]
    fn member_add_payload_carries_guild_id() {
        let member = Member::new(Snowflake::new(1), Snowflake::new(10), at());
        let profile = MemberProfile {
            member,
            user: user(10),
        };
        let routed = route(&DomainEvent::MemberAdd(profile), "nl-home", at());
        assert_eq!(routed[0].envelope.data["guild_id"], "1");
        assert_eq!(routed[0].envelope.data["user"]["id"], "10");
    }

    #[test]
    fn guild_create_is_sent_only_to_recipients() {
        let guild = Guild {
            id: Snowflake::new(1),
            name: "g".into(),
            icon: None,
            description: None,
            owner_id: Snowflake::new(10),
            voice_region: None,
            verification_level: 0,
            default_message_notifications: 0,
            explicit_content_filter: 0,
            afk_channel_id: None,
            afk_timeout: 300,
            system_channel_id: None,
            system_channel_flags: 0,
            banner: None,
            splash: None,
            prune_admin_only: false,
            audit_policy: Default::default(),
            created_at: at(),
        };
        let snapshot = domain::guild::GuildSnapshot {
            guild,
            roles: vec![],
            channels: vec![],
            members: vec![],
            emojis: vec![],
            member_count: 1,
        };
        let event = DomainEvent::GuildCreate {
            snapshot: Box::new(snapshot),
            recipients: vec![Snowflake::new(10)],
        };
        let routed = route(&event, "nl-home", at());
        assert_eq!(
            routed[0].envelope.target_user_ids,
            Some(vec![Id(Snowflake::new(10))])
        );
        assert_eq!(routed[0].envelope.kind, "GUILD_CREATE");
    }

    #[test]
    fn sessions_revoke_targets_the_user_with_the_revoked_ids() {
        let session = uuid::Uuid::from_u128(0xabc);
        let event = DomainEvent::SessionsRevoke {
            user_id: Snowflake::new(10),
            session_ids: vec![session],
        };
        let routed = route(&event, "nl-home", at());
        assert_eq!(routed.len(), 1);
        assert_eq!(routed[0].topic, TOPIC_USER);
        assert_eq!(routed[0].key, "10");
        assert_eq!(routed[0].envelope.kind, "SESSION_REVOKE");
        assert_eq!(
            routed[0].envelope.target_user_ids,
            Some(vec![Id(Snowflake::new(10))])
        );
        assert_eq!(
            routed[0].envelope.data["session_ids"],
            serde_json::json!([session.to_string()])
        );
    }

    #[test]
    fn user_devices_update_is_keyed_by_user_and_targets_the_given_users() {
        let device = domain::device::Device {
            id: Snowflake::new(20),
            user_id: Snowflake::new(10),
            name: "laptop".into(),
            identity_key: [7u8; 32],
            created_at: at(),
            last_seen_at: at(),
        };
        let event = DomainEvent::UserDevicesUpdate {
            user_id: Snowflake::new(10),
            devices: vec![device],
            target_user_ids: vec![Snowflake::new(10), Snowflake::new(11)],
        };
        let routed = route(&event, "nl-home", at());
        assert_eq!(routed.len(), 1);
        assert_eq!(routed[0].topic, TOPIC_USER);
        assert_eq!(routed[0].key, "10");
        assert_eq!(routed[0].envelope.kind, "USER_DEVICES_UPDATE");
        assert_eq!(
            routed[0].envelope.target_user_ids,
            Some(vec![Id(Snowflake::new(10)), Id(Snowflake::new(11))])
        );
        assert_eq!(routed[0].envelope.data["user_id"], "10");
        assert_eq!(routed[0].envelope.data["devices"][0]["device_id"], "20");
        assert!(routed[0].envelope.data["devices"][0]["identity_key_b64"].is_string());
        assert_eq!(
            routed[0].envelope.data["devices"].as_array().map(Vec::len),
            Some(1)
        );
    }
}
