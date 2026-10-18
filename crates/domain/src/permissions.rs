//! Discord-compatible 64-bit permission bitfield and the permission
//! computation algorithm (pinned interface P9).

use std::ops::{BitAnd, BitOr, BitOrAssign, Not};

use crate::snowflake::Snowflake;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Permissions(u64);

macro_rules! permission_bits {
    ($($name:ident = $bit:expr),+ $(,)?) => {
        impl Permissions {
            $(pub const $name: Permissions = Permissions(1 << $bit);)+
            /// Every defined permission bit.
            pub const ALL: Permissions = Permissions(0 $(| (1 << $bit))+);
        }
    };
}

permission_bits! {
    CREATE_INSTANT_INVITE = 0,
    KICK_MEMBERS = 1,
    BAN_MEMBERS = 2,
    ADMINISTRATOR = 3,
    MANAGE_CHANNELS = 4,
    MANAGE_GUILD = 5,
    ADD_REACTIONS = 6,
    VIEW_AUDIT_LOG = 7,
    PRIORITY_SPEAKER = 8,
    STREAM = 9,
    VIEW_CHANNEL = 10,
    SEND_MESSAGES = 11,
    SEND_TTS_MESSAGES = 12,
    MANAGE_MESSAGES = 13,
    EMBED_LINKS = 14,
    ATTACH_FILES = 15,
    READ_MESSAGE_HISTORY = 16,
    MENTION_EVERYONE = 17,
    USE_EXTERNAL_EMOJIS = 18,
    VIEW_GUILD_INSIGHTS = 19,
    CONNECT = 20,
    SPEAK = 21,
    MUTE_MEMBERS = 22,
    DEAFEN_MEMBERS = 23,
    MOVE_MEMBERS = 24,
    USE_VAD = 25,
    CHANGE_NICKNAME = 26,
    MANAGE_NICKNAMES = 27,
    MANAGE_ROLES = 28,
    MANAGE_WEBHOOKS = 29,
    MANAGE_GUILD_EXPRESSIONS = 30,
    USE_APPLICATION_COMMANDS = 31,
    REQUEST_TO_SPEAK = 32,
    MANAGE_EVENTS = 33,
    MANAGE_THREADS = 34,
    CREATE_PUBLIC_THREADS = 35,
    CREATE_PRIVATE_THREADS = 36,
    USE_EXTERNAL_STICKERS = 37,
    SEND_MESSAGES_IN_THREADS = 38,
    USE_EMBEDDED_ACTIVITIES = 39,
    MODERATE_MEMBERS = 40,
    VIEW_CREATOR_MONETIZATION_ANALYTICS = 41,
    USE_SOUNDBOARD = 42,
    CREATE_GUILD_EXPRESSIONS = 43,
    CREATE_EVENTS = 44,
    USE_EXTERNAL_SOUNDS = 45,
    SEND_VOICE_MESSAGES = 46,
    SEND_POLLS = 49,
    USE_EXTERNAL_APPS = 50,
}

impl Permissions {
    pub const NONE: Permissions = Permissions(0);

    /// Permissions that only exist at guild level; they survive a missing
    /// VIEW_CHANNEL.
    pub const GUILD_ONLY: Permissions = Permissions(
        Self::KICK_MEMBERS.0
            | Self::BAN_MEMBERS.0
            | Self::ADMINISTRATOR.0
            | Self::MANAGE_GUILD.0
            | Self::VIEW_AUDIT_LOG.0
            | Self::VIEW_GUILD_INSIGHTS.0
            | Self::CHANGE_NICKNAME.0
            | Self::MANAGE_NICKNAMES.0
            | Self::MANAGE_GUILD_EXPRESSIONS.0
            | Self::MODERATE_MEMBERS.0
            | Self::VIEW_CREATOR_MONETIZATION_ANALYTICS.0
            | Self::CREATE_GUILD_EXPRESSIONS.0,
    );

    /// Permissions that apply inside a channel.
    pub const CHANNEL_SCOPED: Permissions = Permissions(Self::ALL.0 & !Self::GUILD_ONLY.0);

    /// Permissions that depend on SEND_MESSAGES (Discord implicit rule).
    pub const REQUIRES_SEND_MESSAGES: Permissions = Permissions(
        Self::SEND_TTS_MESSAGES.0
            | Self::MENTION_EVERYONE.0
            | Self::ATTACH_FILES.0
            | Self::EMBED_LINKS.0,
    );

    /// What a timed-out member keeps.
    pub const TIMED_OUT_ALLOWED: Permissions =
        Permissions(Self::VIEW_CHANNEL.0 | Self::READ_MESSAGE_HISTORY.0);

    /// @everyone permissions for a freshly created guild.
    pub const DEFAULT_EVERYONE: Permissions = Permissions(
        Self::CREATE_INSTANT_INVITE.0
            | Self::ADD_REACTIONS.0
            | Self::STREAM.0
            | Self::VIEW_CHANNEL.0
            | Self::SEND_MESSAGES.0
            | Self::EMBED_LINKS.0
            | Self::ATTACH_FILES.0
            | Self::READ_MESSAGE_HISTORY.0
            | Self::USE_EXTERNAL_EMOJIS.0
            | Self::CONNECT.0
            | Self::SPEAK.0
            | Self::USE_VAD.0
            | Self::CHANGE_NICKNAME.0
            | Self::USE_APPLICATION_COMMANDS.0
            | Self::REQUEST_TO_SPEAK.0
            | Self::CREATE_PUBLIC_THREADS.0
            | Self::CREATE_PRIVATE_THREADS.0
            | Self::USE_EXTERNAL_STICKERS.0
            | Self::SEND_MESSAGES_IN_THREADS.0
            | Self::USE_EMBEDDED_ACTIVITIES.0
            | Self::USE_SOUNDBOARD.0
            | Self::USE_EXTERNAL_SOUNDS.0
            | Self::SEND_VOICE_MESSAGES.0
            | Self::SEND_POLLS.0
            | Self::USE_EXTERNAL_APPS.0,
    );

    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// Keeps only defined bits.
    pub const fn from_bits_truncate(bits: u64) -> Self {
        Self(bits & Self::ALL.0)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn contains(self, other: Permissions) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Permissions) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn without(self, other: Permissions) -> Self {
        Self(self.0 & !other.0)
    }

    /// Bits set in `self` but missing from `held`.
    pub const fn missing_from(self, held: Permissions) -> Self {
        Self(self.0 & !held.0)
    }
}

impl BitOr for Permissions {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Permissions {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for Permissions {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl Not for Permissions {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverwriteKind {
    Role,
    Member,
}

impl OverwriteKind {
    pub const fn code(self) -> u8 {
        match self {
            OverwriteKind::Role => 0,
            OverwriteKind::Member => 1,
        }
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(OverwriteKind::Role),
            1 => Some(OverwriteKind::Member),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionOverwrite {
    /// Role id (the guild id for @everyone) or user id.
    pub id: Snowflake,
    pub kind: OverwriteKind,
    pub allow: Permissions,
    pub deny: Permissions,
}

/// Permissions granted by one role of the guild.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RolePermissions {
    pub role_id: Snowflake,
    pub permissions: Permissions,
}

/// Guild-level facts needed to compute anyone's permissions.
#[derive(Debug, Clone, Copy)]
pub struct GuildPermissionContext<'a> {
    /// Also the id of the @everyone role.
    pub guild_id: Snowflake,
    pub owner_id: Snowflake,
    pub roles: &'a [RolePermissions],
}

/// The member whose permissions are computed.
#[derive(Debug, Clone, Copy)]
pub struct MemberPermissionContext<'a> {
    pub user_id: Snowflake,
    pub role_ids: &'a [Snowflake],
    pub timed_out: bool,
}

impl GuildPermissionContext<'_> {
    fn role_permissions(&self, role_id: Snowflake) -> Permissions {
        self.roles
            .iter()
            .find(|role| role.role_id == role_id)
            .map(|role| role.permissions)
            .unwrap_or(Permissions::NONE)
    }

    /// Steps 1–2: @everyone OR member roles; owner and ADMINISTRATOR get all.
    pub fn base_permissions(&self, member: &MemberPermissionContext<'_>) -> Permissions {
        if member.user_id == self.owner_id {
            return Permissions::ALL;
        }
        let mut permissions = self.role_permissions(self.guild_id);
        for role_id in member.role_ids {
            permissions |= self.role_permissions(*role_id);
        }
        if permissions.contains(Permissions::ADMINISTRATOR) {
            return Permissions::ALL;
        }
        if member.timed_out {
            return permissions & Permissions::TIMED_OUT_ALLOWED;
        }
        permissions
    }

    /// Steps 1–4 for a channel with the given overwrites.
    pub fn channel_permissions(
        &self,
        member: &MemberPermissionContext<'_>,
        overwrites: &[PermissionOverwrite],
    ) -> Permissions {
        let base = self.base_permissions(member);
        if base == Permissions::ALL {
            return Permissions::ALL;
        }
        let mut permissions = self.apply_overwrites(base, member, overwrites);
        if member.timed_out {
            permissions = permissions & Permissions::TIMED_OUT_ALLOWED;
        }
        apply_implicit_rules(permissions)
    }

    fn apply_overwrites(
        &self,
        base: Permissions,
        member: &MemberPermissionContext<'_>,
        overwrites: &[PermissionOverwrite],
    ) -> Permissions {
        let mut permissions = base;

        if let Some(everyone) = overwrites
            .iter()
            .find(|o| o.kind == OverwriteKind::Role && o.id == self.guild_id)
        {
            permissions = permissions.without(everyone.deny) | everyone.allow;
        }

        let (mut role_allow, mut role_deny) = (Permissions::NONE, Permissions::NONE);
        for overwrite in overwrites
            .iter()
            .filter(|o| o.kind == OverwriteKind::Role && o.id != self.guild_id)
            .filter(|o| member.role_ids.contains(&o.id))
        {
            role_allow |= overwrite.allow;
            role_deny |= overwrite.deny;
        }
        permissions = permissions.without(role_deny) | role_allow;

        if let Some(own) = overwrites
            .iter()
            .find(|o| o.kind == OverwriteKind::Member && o.id == member.user_id)
        {
            permissions = permissions.without(own.deny) | own.allow;
        }
        permissions
    }
}

/// Step 4 plus Discord's implicit text rule.
fn apply_implicit_rules(permissions: Permissions) -> Permissions {
    if !permissions.contains(Permissions::VIEW_CHANNEL) {
        return permissions.without(Permissions::CHANNEL_SCOPED);
    }
    if !permissions.contains(Permissions::SEND_MESSAGES) {
        return permissions.without(Permissions::REQUIRES_SEND_MESSAGES);
    }
    permissions
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUILD: Snowflake = Snowflake::new(100);
    const OWNER: Snowflake = Snowflake::new(1);
    const ALICE: Snowflake = Snowflake::new(2);
    const BOB: Snowflake = Snowflake::new(3);
    const MOD_ROLE: Snowflake = Snowflake::new(200);
    const MUTED_ROLE: Snowflake = Snowflake::new(201);
    const ADMIN_ROLE: Snowflake = Snowflake::new(202);
    const OTHER_ROLE: Snowflake = Snowflake::new(203);
    const N: Permissions = Permissions::NONE;

    fn roles() -> Vec<RolePermissions> {
        vec![
            RolePermissions {
                role_id: GUILD,
                permissions: everyone_base(),
            },
            RolePermissions {
                role_id: MOD_ROLE,
                permissions: Permissions::KICK_MEMBERS | Permissions::MANAGE_MESSAGES,
            },
            RolePermissions {
                role_id: MUTED_ROLE,
                permissions: N,
            },
            RolePermissions {
                role_id: ADMIN_ROLE,
                permissions: Permissions::ADMINISTRATOR,
            },
            RolePermissions {
                role_id: OTHER_ROLE,
                permissions: Permissions::MANAGE_ROLES,
            },
        ]
    }

    fn everyone_base() -> Permissions {
        Permissions::VIEW_CHANNEL
            | Permissions::SEND_MESSAGES
            | Permissions::READ_MESSAGE_HISTORY
            | Permissions::EMBED_LINKS
            | Permissions::CHANGE_NICKNAME
    }

    fn role_ow(id: Snowflake, allow: Permissions, deny: Permissions) -> PermissionOverwrite {
        PermissionOverwrite {
            id,
            kind: OverwriteKind::Role,
            allow,
            deny,
        }
    }

    fn member_ow(id: Snowflake, allow: Permissions, deny: Permissions) -> PermissionOverwrite {
        PermissionOverwrite {
            id,
            kind: OverwriteKind::Member,
            allow,
            deny,
        }
    }

    struct Case {
        name: &'static str,
        user: Snowflake,
        roles: Vec<Snowflake>,
        timed_out: bool,
        overwrites: Vec<PermissionOverwrite>,
        has: Permissions,
        lacks: Permissions,
    }

    fn case(
        name: &'static str,
        user: Snowflake,
        roles: Vec<Snowflake>,
        overwrites: Vec<PermissionOverwrite>,
        has: Permissions,
        lacks: Permissions,
    ) -> Case {
        Case {
            name,
            user,
            roles,
            timed_out: false,
            overwrites,
            has,
            lacks,
        }
    }

    fn cases() -> Vec<Case> {
        vec![
            case(
                "owner gets everything without roles",
                OWNER,
                vec![],
                vec![role_ow(GUILD, N, Permissions::VIEW_CHANNEL)],
                Permissions::ALL,
                N,
            ),
            Case {
                timed_out: true,
                ..case(
                    "owner ignores timeout",
                    OWNER,
                    vec![],
                    vec![],
                    Permissions::ALL,
                    N,
                )
            },
            case(
                "administrator role grants all and bypasses overwrites",
                ALICE,
                vec![ADMIN_ROLE],
                vec![
                    role_ow(GUILD, N, Permissions::VIEW_CHANNEL),
                    member_ow(ALICE, N, Permissions::SEND_MESSAGES),
                ],
                Permissions::ALL,
                N,
            ),
            case(
                "everyone base applies without overwrites",
                ALICE,
                vec![],
                vec![],
                everyone_base(),
                Permissions::KICK_MEMBERS | Permissions::MANAGE_MESSAGES,
            ),
            case(
                "member roles are ORed into base",
                ALICE,
                vec![MOD_ROLE],
                vec![],
                everyone_base() | Permissions::KICK_MEMBERS | Permissions::MANAGE_MESSAGES,
                Permissions::ADMINISTRATOR,
            ),
            case(
                "everyone overwrite deny removes a base permission",
                ALICE,
                vec![],
                vec![role_ow(GUILD, N, Permissions::SEND_MESSAGES)],
                Permissions::VIEW_CHANNEL,
                Permissions::SEND_MESSAGES,
            ),
            case(
                "everyone overwrite allow adds a permission",
                ALICE,
                vec![],
                vec![role_ow(GUILD, Permissions::ATTACH_FILES, N)],
                Permissions::ATTACH_FILES,
                N,
            ),
            case(
                "role overwrite allow beats everyone overwrite deny",
                ALICE,
                vec![MOD_ROLE],
                vec![
                    role_ow(GUILD, N, Permissions::SEND_MESSAGES),
                    role_ow(MOD_ROLE, Permissions::SEND_MESSAGES, N),
                ],
                Permissions::SEND_MESSAGES,
                N,
            ),
            case(
                "role overwrites apply all denies before all allows",
                ALICE,
                vec![MOD_ROLE, MUTED_ROLE],
                vec![
                    role_ow(MUTED_ROLE, N, Permissions::SEND_MESSAGES),
                    role_ow(MOD_ROLE, Permissions::SEND_MESSAGES, N),
                ],
                Permissions::SEND_MESSAGES,
                N,
            ),
            case(
                "role overwrite order in the list does not matter",
                ALICE,
                vec![MOD_ROLE, MUTED_ROLE],
                vec![
                    role_ow(MOD_ROLE, Permissions::SEND_MESSAGES, N),
                    role_ow(MUTED_ROLE, N, Permissions::SEND_MESSAGES),
                ],
                Permissions::SEND_MESSAGES,
                N,
            ),
            case(
                "role overwrite deny applies when no role allows",
                ALICE,
                vec![MUTED_ROLE],
                vec![role_ow(MUTED_ROLE, N, Permissions::SEND_MESSAGES)],
                Permissions::VIEW_CHANNEL,
                Permissions::SEND_MESSAGES,
            ),
            case(
                "member overwrite deny beats role overwrite allow",
                ALICE,
                vec![MOD_ROLE],
                vec![
                    role_ow(MOD_ROLE, Permissions::ATTACH_FILES, N),
                    member_ow(ALICE, N, Permissions::ATTACH_FILES),
                ],
                Permissions::VIEW_CHANNEL,
                Permissions::ATTACH_FILES,
            ),
            case(
                "member overwrite allow beats role and everyone deny",
                ALICE,
                vec![MUTED_ROLE],
                vec![
                    role_ow(GUILD, N, Permissions::SEND_MESSAGES),
                    role_ow(MUTED_ROLE, N, Permissions::SEND_MESSAGES),
                    member_ow(ALICE, Permissions::SEND_MESSAGES, N),
                ],
                Permissions::SEND_MESSAGES,
                N,
            ),
            case(
                "overwrite for a role the member lacks is ignored",
                ALICE,
                vec![],
                vec![role_ow(
                    MOD_ROLE,
                    Permissions::MANAGE_MESSAGES,
                    Permissions::SEND_MESSAGES,
                )],
                Permissions::SEND_MESSAGES,
                Permissions::MANAGE_MESSAGES,
            ),
            case(
                "member overwrite for someone else is ignored",
                ALICE,
                vec![],
                vec![member_ow(BOB, N, Permissions::SEND_MESSAGES)],
                Permissions::SEND_MESSAGES,
                N,
            ),
            case(
                "member-type overwrite carrying a role id is not a role overwrite",
                ALICE,
                vec![MOD_ROLE],
                vec![member_ow(MOD_ROLE, N, Permissions::SEND_MESSAGES)],
                Permissions::SEND_MESSAGES,
                N,
            ),
            case(
                "missing VIEW_CHANNEL strips channel permissions but keeps guild ones",
                ALICE,
                vec![MOD_ROLE],
                vec![role_ow(GUILD, N, Permissions::VIEW_CHANNEL)],
                Permissions::KICK_MEMBERS | Permissions::CHANGE_NICKNAME,
                Permissions::VIEW_CHANNEL
                    | Permissions::SEND_MESSAGES
                    | Permissions::MANAGE_MESSAGES,
            ),
            case(
                "VIEW_CHANNEL regained through a role overwrite",
                ALICE,
                vec![MOD_ROLE],
                vec![
                    role_ow(GUILD, N, Permissions::VIEW_CHANNEL),
                    role_ow(MOD_ROLE, Permissions::VIEW_CHANNEL, N),
                ],
                Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
                N,
            ),
            case(
                "member overwrite deny of VIEW_CHANNEL hides the channel",
                ALICE,
                vec![MOD_ROLE],
                vec![
                    role_ow(MOD_ROLE, Permissions::VIEW_CHANNEL, N),
                    member_ow(ALICE, N, Permissions::VIEW_CHANNEL),
                ],
                Permissions::KICK_MEMBERS,
                Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY,
            ),
            case(
                "missing SEND_MESSAGES strips embed, attach, tts and mention everyone",
                ALICE,
                vec![],
                vec![role_ow(
                    GUILD,
                    Permissions::ATTACH_FILES | Permissions::MENTION_EVERYONE,
                    Permissions::SEND_MESSAGES,
                )],
                Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY,
                Permissions::REQUIRES_SEND_MESSAGES,
            ),
            Case {
                timed_out: true,
                ..case(
                    "timed-out member keeps only view and history",
                    ALICE,
                    vec![MOD_ROLE],
                    vec![member_ow(ALICE, Permissions::SEND_MESSAGES, N)],
                    Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY,
                    Permissions::SEND_MESSAGES | Permissions::KICK_MEMBERS,
                )
            },
            Case {
                timed_out: true,
                ..case(
                    "timed-out administrator is unaffected",
                    ALICE,
                    vec![ADMIN_ROLE],
                    vec![],
                    Permissions::ALL,
                    N,
                )
            },
            case(
                "role overwrite can deny a permission granted by another role",
                ALICE,
                vec![OTHER_ROLE],
                vec![role_ow(
                    OTHER_ROLE,
                    Permissions::MANAGE_CHANNELS,
                    Permissions::MANAGE_ROLES,
                )],
                Permissions::MANAGE_CHANNELS,
                Permissions::MANAGE_ROLES,
            ),
        ]
    }

    #[test]
    fn channel_permission_table() {
        let roles = roles();
        let guild = GuildPermissionContext {
            guild_id: GUILD,
            owner_id: OWNER,
            roles: &roles,
        };
        for case in cases() {
            let member = MemberPermissionContext {
                user_id: case.user,
                role_ids: &case.roles,
                timed_out: case.timed_out,
            };
            let computed = guild.channel_permissions(&member, &case.overwrites);
            assert!(
                computed.contains(case.has),
                "{}: missing {:#x}",
                case.name,
                case.has.missing_from(computed).bits()
            );
            assert!(
                !computed.intersects(case.lacks),
                "{}: unexpectedly has {:#x}",
                case.name,
                (computed & case.lacks).bits()
            );
        }
    }

    #[test]
    fn base_permission_table() {
        let roles = roles();
        let guild = GuildPermissionContext {
            guild_id: GUILD,
            owner_id: OWNER,
            roles: &roles,
        };
        let table: Vec<(&str, Snowflake, Vec<Snowflake>, bool, Permissions)> = vec![
            ("owner", OWNER, vec![], false, Permissions::ALL),
            ("everyone only", ALICE, vec![], false, everyone_base()),
            (
                "with mod role",
                ALICE,
                vec![MOD_ROLE],
                false,
                everyone_base() | Permissions::KICK_MEMBERS | Permissions::MANAGE_MESSAGES,
            ),
            ("admin", ALICE, vec![ADMIN_ROLE], false, Permissions::ALL),
            (
                "unknown role ignored",
                ALICE,
                vec![Snowflake::new(999)],
                false,
                everyone_base(),
            ),
            (
                "timed out",
                ALICE,
                vec![MOD_ROLE],
                true,
                Permissions::TIMED_OUT_ALLOWED,
            ),
        ];
        for (name, user, role_ids, timed_out, expected) in table {
            let member = MemberPermissionContext {
                user_id: user,
                role_ids: &role_ids,
                timed_out,
            };
            assert_eq!(guild.base_permissions(&member), expected, "{name}");
        }
    }

    #[test]
    fn exact_result_for_member_overwrite() {
        let roles = roles();
        let guild = GuildPermissionContext {
            guild_id: GUILD,
            owner_id: OWNER,
            roles: &roles,
        };
        let member = MemberPermissionContext {
            user_id: BOB,
            role_ids: &[],
            timed_out: false,
        };
        let computed = guild.channel_permissions(
            &member,
            &[member_ow(
                BOB,
                Permissions::CONNECT,
                Permissions::EMBED_LINKS,
            )],
        );
        assert_eq!(
            computed,
            Permissions::VIEW_CHANNEL
                | Permissions::SEND_MESSAGES
                | Permissions::READ_MESSAGE_HISTORY
                | Permissions::CHANGE_NICKNAME
                | Permissions::CONNECT
        );
    }

    #[test]
    fn discord_bit_values() {
        assert_eq!(Permissions::ADMINISTRATOR.bits(), 8);
        assert_eq!(Permissions::VIEW_CHANNEL.bits(), 1024);
        assert_eq!(Permissions::CONNECT.bits(), 1_048_576);
        assert_eq!(Permissions::STREAM.bits(), 512);
        assert_eq!(Permissions::USE_APPLICATION_COMMANDS.bits(), 1 << 31);
        assert_eq!(Permissions::SEND_MESSAGES_IN_THREADS.bits(), 1 << 38);
        assert!(!Permissions::ALL.contains(Permissions::from_bits(1 << 47)));
    }

    #[test]
    fn truncates_unknown_bits() {
        assert_eq!(Permissions::from_bits_truncate(u64::MAX), Permissions::ALL);
    }

    #[test]
    fn channel_scoped_and_guild_only_partition_all() {
        assert_eq!(
            Permissions::CHANNEL_SCOPED | Permissions::GUILD_ONLY,
            Permissions::ALL
        );
        assert!(!Permissions::CHANNEL_SCOPED.intersects(Permissions::GUILD_ONLY));
    }
}
