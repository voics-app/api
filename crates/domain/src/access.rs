//! Guild access: applies the permission algorithm to the domain entities,
//! including role hierarchy and thread inheritance.

use chrono::{DateTime, Utc};

use crate::channel::{Channel, ChannelType};
use crate::guild::{Guild, Member, Role};
use crate::permissions::{
    GuildPermissionContext, MemberPermissionContext, Permissions, RolePermissions,
};
use crate::snowflake::Snowflake;

pub struct GuildAccess<'a> {
    guild: &'a Guild,
    roles: &'a [Role],
    role_permissions: Vec<RolePermissions>,
    now: DateTime<Utc>,
}

impl<'a> GuildAccess<'a> {
    pub fn new(guild: &'a Guild, roles: &'a [Role], now: DateTime<Utc>) -> Self {
        let role_permissions = roles
            .iter()
            .map(|r| RolePermissions {
                role_id: r.id,
                permissions: r.permissions,
            })
            .collect();
        Self {
            guild,
            roles,
            role_permissions,
            now,
        }
    }

    fn context(&self) -> GuildPermissionContext<'_> {
        GuildPermissionContext {
            guild_id: self.guild.id,
            owner_id: self.guild.owner_id,
            roles: &self.role_permissions,
        }
    }

    fn member_context<'m>(&self, member: &'m Member) -> MemberPermissionContext<'m> {
        MemberPermissionContext {
            user_id: member.user_id,
            role_ids: &member.role_ids,
            timed_out: member.is_timed_out(self.now),
        }
    }

    pub fn is_owner(&self, user_id: Snowflake) -> bool {
        self.guild.owner_id == user_id
    }

    /// Guild-wide permissions.
    pub fn base(&self, member: &Member) -> Permissions {
        self.context()
            .base_permissions(&self.member_context(member))
    }

    /// Permissions inside `channel`. Threads inherit `parent`'s overwrites.
    pub fn in_channel(
        &self,
        member: &Member,
        channel: &Channel,
        parent: Option<&Channel>,
    ) -> Permissions {
        let overwrites = match (channel.kind.is_thread(), parent) {
            (true, Some(parent)) => &parent.overwrites,
            _ => &channel.overwrites,
        };
        let permissions = self
            .context()
            .channel_permissions(&self.member_context(member), overwrites);
        let permissions = if channel.kind.is_voice() && !channel.text_chat_enabled {
            permissions.without(Permissions::SEND_MESSAGES)
        } else {
            permissions
        };
        if !channel.kind.is_thread() || permissions == Permissions::ALL {
            return permissions;
        }
        self.thread_permissions(member, channel, permissions)
    }

    /// In threads SEND_MESSAGES_IN_THREADS replaces SEND_MESSAGES, and private
    /// threads are visible only to their creator and thread managers.
    fn thread_permissions(
        &self,
        member: &Member,
        thread: &Channel,
        permissions: Permissions,
    ) -> Permissions {
        let private_hidden = thread.kind == ChannelType::PrivateThread
            && thread.owner_id != Some(member.user_id)
            && !permissions.contains(Permissions::MANAGE_THREADS);
        if private_hidden {
            return permissions.without(Permissions::CHANNEL_SCOPED);
        }
        let mut adjusted = permissions.without(Permissions::SEND_MESSAGES);
        if permissions.contains(Permissions::SEND_MESSAGES_IN_THREADS) {
            adjusted |= Permissions::SEND_MESSAGES;
        }
        adjusted
    }

    /// Position of the member's highest role (@everyone is 0).
    pub fn highest_position(&self, member: &Member) -> i32 {
        self.roles
            .iter()
            .filter(|role| member.role_ids.contains(&role.id))
            .map(|role| role.position)
            .max()
            .unwrap_or(0)
    }

    /// Whether `actor` may moderate `target` (kick, ban, edit roles/nick).
    pub fn outranks(&self, actor: &Member, target: &Member) -> bool {
        if self.is_owner(target.user_id) {
            return false;
        }
        if self.is_owner(actor.user_id) {
            return true;
        }
        self.highest_position(actor) > self.highest_position(target)
    }

    /// Whether `actor` may edit, assign or delete `role`.
    pub fn can_manage_role(&self, actor: &Member, role: &Role) -> bool {
        self.is_owner(actor.user_id) || self.highest_position(actor) > role.position
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{OverwriteKind, PermissionOverwrite};
    use chrono::Duration;

    const GUILD: Snowflake = Snowflake::new(10);
    const OWNER: Snowflake = Snowflake::new(1);

    fn guild() -> Guild {
        Guild {
            id: GUILD,
            name: "g".into(),
            icon: None,
            description: None,
            owner_id: OWNER,
            voice_region: None,
            verification_level: 0,
            default_message_notifications: 0,
            explicit_content_filter: 0,
            afk_channel_id: None,
            afk_timeout: 300,
            system_channel_id: None,
            created_at: Utc::now(),
        }
    }

    fn role(id: u64, position: i32, permissions: Permissions) -> Role {
        Role {
            id: Snowflake::new(id),
            position,
            permissions,
            ..Role::everyone(GUILD, permissions)
        }
    }

    fn roles() -> Vec<Role> {
        vec![
            Role::everyone(
                GUILD,
                Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES_IN_THREADS,
            ),
            role(20, 1, Permissions::KICK_MEMBERS),
            role(21, 2, Permissions::MANAGE_ROLES),
            role(22, 3, Permissions::MANAGE_THREADS),
        ]
    }

    fn member(user: u64, roles: &[u64]) -> Member {
        Member {
            role_ids: roles.iter().map(|r| Snowflake::new(*r)).collect(),
            ..Member::new(GUILD, Snowflake::new(user), Utc::now())
        }
    }

    fn channel(id: u64, kind: ChannelType) -> Channel {
        Channel::new(Snowflake::new(id), kind, Some(GUILD), Utc::now())
    }

    #[test]
    fn hierarchy_table() {
        let (guild, roles) = (guild(), roles());
        let access = GuildAccess::new(&guild, &roles, Utc::now());
        let owner = member(1, &[]);
        let low = member(2, &[20]);
        let high = member(3, &[21]);
        let peer = member(4, &[21]);
        let cases = [
            ("owner outranks anyone", &owner, &high, true),
            ("nobody outranks the owner", &high, &owner, false),
            ("higher role outranks lower", &high, &low, true),
            ("lower role cannot act on higher", &low, &high, false),
            ("equal positions do not outrank", &high, &peer, false),
        ];
        for (name, actor, target, expected) in cases {
            assert_eq!(access.outranks(actor, target), expected, "{name}");
        }
        assert!(access.can_manage_role(&high, &roles[1]));
        assert!(!access.can_manage_role(&high, &roles[2]));
        assert!(access.can_manage_role(&owner, &roles[3]));
        assert_eq!(access.highest_position(&member(5, &[])), 0);
    }

    #[test]
    fn threads_inherit_parent_overwrites_and_map_send_permission() {
        let (guild, roles) = (guild(), roles());
        let access = GuildAccess::new(&guild, &roles, Utc::now());
        let mut parent = channel(100, ChannelType::GuildText);
        parent.overwrites.push(PermissionOverwrite {
            id: Snowflake::new(2),
            kind: OverwriteKind::Member,
            allow: Permissions::READ_MESSAGE_HISTORY,
            deny: Permissions::NONE,
        });
        let mut thread = channel(101, ChannelType::PublicThread);
        thread.parent_id = Some(parent.id);
        let alice = member(2, &[]);
        let perms = access.in_channel(&alice, &thread, Some(&parent));
        assert!(perms.contains(Permissions::READ_MESSAGE_HISTORY));
        assert!(perms.contains(Permissions::SEND_MESSAGES));
        assert!(!access
            .in_channel(&alice, &parent, None)
            .contains(Permissions::SEND_MESSAGES));
    }

    #[test]
    fn voice_text_chat_can_be_switched_off_for_senders() {
        let (guild, roles) = (guild(), roles());
        let access = GuildAccess::new(&guild, &roles, Utc::now());
        let mut voice = channel(102, ChannelType::GuildVoice);
        let alice = member(2, &[]);
        let owner = member(1, &[]);
        assert!(voice.text_chat_enabled);
        // Text chat is on by default: the everyone role grants sending here.
        voice.overwrites.push(PermissionOverwrite {
            id: GUILD,
            kind: OverwriteKind::Role,
            allow: Permissions::SEND_MESSAGES | Permissions::VIEW_CHANNEL,
            deny: Permissions::NONE,
        });
        assert!(access
            .in_channel(&alice, &voice, None)
            .contains(Permissions::SEND_MESSAGES));
        voice.text_chat_enabled = false;
        for m in [&alice, &owner] {
            let perms = access.in_channel(m, &voice, None);
            assert!(
                !perms.contains(Permissions::SEND_MESSAGES),
                "user {}",
                m.user_id
            );
            assert!(
                perms.contains(Permissions::VIEW_CHANNEL),
                "user {}",
                m.user_id
            );
        }
    }

    #[test]
    fn private_threads_visible_to_creator_and_managers_only() {
        let (guild, roles) = (guild(), roles());
        let access = GuildAccess::new(&guild, &roles, Utc::now());
        let parent = channel(100, ChannelType::GuildText);
        let mut thread = channel(101, ChannelType::PrivateThread);
        thread.owner_id = Some(Snowflake::new(2));
        let cases = [
            (member(2, &[]), true),
            (member(3, &[]), false),
            (member(4, &[22]), true),
            (member(1, &[]), true),
        ];
        for (m, visible) in cases {
            let perms = access.in_channel(&m, &thread, Some(&parent));
            assert_eq!(
                perms.contains(Permissions::VIEW_CHANNEL),
                visible,
                "user {}",
                m.user_id
            );
        }
    }

    #[test]
    fn timeout_applies_through_member_state() {
        let (guild, roles) = (guild(), roles());
        let now = Utc::now();
        let access = GuildAccess::new(&guild, &roles, now);
        let mut m = member(2, &[20]);
        m.communication_disabled_until = Some(now + Duration::minutes(5));
        assert!(!access.base(&m).contains(Permissions::KICK_MEMBERS));
        m.communication_disabled_until = Some(now - Duration::minutes(5));
        assert!(access.base(&m).contains(Permissions::KICK_MEMBERS));
    }
}
