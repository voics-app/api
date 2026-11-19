//! Guild, role, member and ban reads.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::FromRow;

use application::ports::{DiscoveryEntry, DiscoveryPage, DiscoveryQuery, GuildRepository};
use application::AppResult;
use domain::discovery::GuildDiscovery;
use domain::guild::{Ban, Guild, Member, MemberProfile, Role};
use domain::template::{GuildTemplate, TemplateSource};
use domain::Snowflake;

use super::convert::{like_pattern, permissions, sf, sf_opt, sfs};
use super::error::db;
use super::users::{UserRow, USER_COLUMNS};
use super::PgStore;

const GUILD_COLUMNS: &str = "id, name, icon, description, owner_id, voice_region, verification_level, \
     default_message_notifications, explicit_content_filter, afk_channel_id, afk_timeout, system_channel_id, system_channel_flags, banner, splash, prune_admin_only, audit_log_enabled, audit_log_retention_days, rules_channel_id, public_updates_channel_id, created_at";

pub(super) const ROLE_COLUMNS: &str =
    "id, guild_id, name, color, hoist, icon, unicode_emoji, position, permissions, \
     managed, mentionable, bot_id";

/// Member columns, with the role list aggregated from `member_roles`.
pub(super) const MEMBER_COLUMNS: &str = "m.guild_id, m.user_id, m.nick, m.avatar AS member_avatar, m.banner AS member_banner, m.bio AS member_bio, m.joined_at, m.deaf, m.mute, \
     m.communication_disabled_until, m.pending, \
     ARRAY(SELECT role_id FROM member_roles mr WHERE mr.guild_id = m.guild_id AND mr.user_id = m.user_id \
           ORDER BY role_id) AS role_ids";

#[derive(FromRow)]
pub(super) struct GuildRow {
    id: i64,
    name: String,
    icon: Option<String>,
    description: Option<String>,
    owner_id: i64,
    voice_region: Option<String>,
    verification_level: i16,
    default_message_notifications: i16,
    explicit_content_filter: i16,
    afk_channel_id: Option<i64>,
    afk_timeout: i32,
    system_channel_id: Option<i64>,
    system_channel_flags: i32,
    banner: Option<String>,
    splash: Option<String>,
    prune_admin_only: bool,
    audit_log_enabled: bool,
    audit_log_retention_days: i16,
    rules_channel_id: Option<i64>,
    public_updates_channel_id: Option<i64>,
    created_at: DateTime<Utc>,
}

impl From<GuildRow> for Guild {
    fn from(row: GuildRow) -> Self {
        Guild {
            id: sf(row.id),
            name: row.name,
            icon: row.icon,
            description: row.description,
            owner_id: sf(row.owner_id),
            voice_region: row.voice_region,
            verification_level: row.verification_level as u8,
            default_message_notifications: row.default_message_notifications as u8,
            explicit_content_filter: row.explicit_content_filter as u8,
            afk_channel_id: sf_opt(row.afk_channel_id),
            afk_timeout: row.afk_timeout as u32,
            system_channel_id: sf_opt(row.system_channel_id),
            system_channel_flags: row.system_channel_flags as u32,
            banner: row.banner,
            splash: row.splash,
            prune_admin_only: row.prune_admin_only,
            community: row.rules_channel_id.zip(row.public_updates_channel_id).map(
                |(rules, updates)| domain::guild::CommunityChannels {
                    rules_channel_id: sf(rules),
                    public_updates_channel_id: sf(updates),
                },
            ),
            audit_policy: domain::audit_policy::AuditPolicy {
                enabled: row.audit_log_enabled,
                retention_days: row.audit_log_retention_days as u16,
            },
            created_at: row.created_at,
        }
    }
}

#[derive(FromRow)]
pub(super) struct RoleRow {
    id: i64,
    guild_id: i64,
    name: String,
    color: i32,
    hoist: bool,
    icon: Option<String>,
    unicode_emoji: Option<String>,
    position: i32,
    permissions: i64,
    managed: bool,
    mentionable: bool,
    bot_id: Option<i64>,
}

impl From<RoleRow> for Role {
    fn from(row: RoleRow) -> Self {
        Role {
            id: sf(row.id),
            guild_id: sf(row.guild_id),
            name: row.name,
            color: row.color as u32,
            hoist: row.hoist,
            icon: row.icon,
            unicode_emoji: row.unicode_emoji,
            position: row.position,
            permissions: permissions(row.permissions),
            managed: row.managed,
            mentionable: row.mentionable,
            bot_id: sf_opt(row.bot_id),
        }
    }
}

#[derive(FromRow)]
pub(super) struct MemberRow {
    guild_id: i64,
    user_id: i64,
    nick: Option<String>,
    #[sqlx(rename = "member_avatar")]
    avatar: Option<String>,
    #[sqlx(rename = "member_banner")]
    banner: Option<String>,
    #[sqlx(rename = "member_bio")]
    bio: Option<String>,
    joined_at: DateTime<Utc>,
    deaf: bool,
    mute: bool,
    communication_disabled_until: Option<DateTime<Utc>>,
    pending: bool,
    role_ids: Vec<i64>,
}

impl From<MemberRow> for Member {
    fn from(row: MemberRow) -> Self {
        Member {
            guild_id: sf(row.guild_id),
            user_id: sf(row.user_id),
            nick: row.nick,
            avatar: row.avatar,
            banner: row.banner,
            bio: row.bio,
            role_ids: sfs(row.role_ids),
            joined_at: row.joined_at,
            deaf: row.deaf,
            mute: row.mute,
            communication_disabled_until: row.communication_disabled_until,
            pending: row.pending,
        }
    }
}

/// A member row joined with its user, as produced by `member_profile_query`.
#[derive(FromRow)]
struct MemberProfileRow {
    #[sqlx(flatten)]
    member: MemberRow,
    #[sqlx(flatten)]
    user: UserRow,
}

impl From<MemberProfileRow> for MemberProfile {
    fn from(row: MemberProfileRow) -> Self {
        MemberProfile {
            member: row.member.into(),
            user: row.user.into(),
        }
    }
}

#[derive(FromRow)]
struct BanRow {
    guild_id: i64,
    reason: Option<String>,
    #[sqlx(flatten)]
    user: UserRow,
}

impl From<BanRow> for Ban {
    fn from(row: BanRow) -> Self {
        Ban {
            guild_id: sf(row.guild_id),
            user: row.user.into(),
            reason: row.reason,
        }
    }
}

fn member_profile_select(filter: &str) -> String {
    format!(
        "SELECT {MEMBER_COLUMNS}, {user_columns} FROM members m JOIN users u ON u.id = m.user_id {filter}",
        user_columns = prefixed_user_columns()
    )
}

pub(super) async fn committed_member(
    conn: &mut sqlx::PgConnection,
    member: &Member,
) -> AppResult<MemberProfile> {
    let row: MemberProfileRow = sqlx::query_as(&member_profile_select(
        "WHERE m.guild_id = $1 AND m.user_id = $2",
    ))
    .bind(member.guild_id.as_i64())
    .bind(member.user_id.as_i64())
    .fetch_one(conn)
    .await
    .map_err(db)?;
    Ok(row.into())
}

#[async_trait]
impl GuildRepository for PgStore {
    async fn welcome_screen(&self, guild_id: Snowflake) -> AppResult<serde_json::Value> {
        let (value,): (serde_json::Value,) =
            sqlx::query_as("SELECT welcome_screen FROM guild_member_screening WHERE guild_id = $1")
                .bind(guild_id.as_i64())
                .fetch_one(&self.pool)
                .await
                .map_err(db)?;
        Ok(value)
    }

    async fn set_welcome_screen(
        &self,
        guild_id: Snowflake,
        value: &serde_json::Value,
    ) -> AppResult<()> {
        sqlx::query("UPDATE guild_member_screening SET welcome_screen = $2 WHERE guild_id = $1")
            .bind(guild_id.as_i64())
            .bind(value)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }
    async fn screening(
        &self,
        guild_id: Snowflake,
    ) -> AppResult<application::ports::ScreeningConfig> {
        let (enabled, rules, revision): (bool, Vec<String>, i64) = sqlx::query_as(
            "SELECT enabled, rules, revision FROM guild_member_screening WHERE guild_id = $1",
        )
        .bind(guild_id.as_i64())
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;
        Ok(application::ports::ScreeningConfig {
            enabled,
            rules,
            revision,
        })
    }

    async fn set_screening(
        &self,
        guild_id: Snowflake,
        enabled: bool,
        rules: &[String],
    ) -> AppResult<application::ports::ScreeningConfig> {
        let (enabled, rules, revision): (bool, Vec<String>, i64) = sqlx::query_as("UPDATE guild_member_screening SET enabled = $2, rules = $3, revision = revision + 1 WHERE guild_id = $1 RETURNING enabled, rules, revision")
            .bind(guild_id.as_i64()).bind(enabled).bind(rules).fetch_one(&self.pool).await.map_err(db)?;
        Ok(application::ports::ScreeningConfig {
            enabled,
            rules,
            revision,
        })
    }
    async fn find(&self, id: Snowflake) -> AppResult<Option<Guild>> {
        let sql = format!("SELECT {GUILD_COLUMNS} FROM guilds WHERE id = $1");
        let row: Option<GuildRow> = sqlx::query_as(&sql)
            .bind(id.as_i64())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        Ok(row.map(Guild::from))
    }

    async fn list_for_user(&self, user_id: Snowflake) -> AppResult<Vec<Guild>> {
        let sql = format!(
            "SELECT g.{cols} FROM guilds g JOIN members m ON m.guild_id = g.id WHERE m.user_id = $1 ORDER BY g.id",
            cols = GUILD_COLUMNS.replace(", ", ", g.")
        );
        let rows: Vec<GuildRow> = sqlx::query_as(&sql)
            .bind(user_id.as_i64())
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        Ok(rows.into_iter().map(Guild::from).collect())
    }

    async fn roles(&self, guild_id: Snowflake) -> AppResult<Vec<Role>> {
        let sql =
            format!("SELECT {ROLE_COLUMNS} FROM roles WHERE guild_id = $1 ORDER BY position, id");
        let rows: Vec<RoleRow> = sqlx::query_as(&sql)
            .bind(guild_id.as_i64())
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        Ok(rows.into_iter().map(Role::from).collect())
    }

    async fn member(&self, guild_id: Snowflake, user_id: Snowflake) -> AppResult<Option<Member>> {
        let sql = format!(
            "SELECT {MEMBER_COLUMNS} FROM members m WHERE m.guild_id = $1 AND m.user_id = $2"
        );
        let row: Option<MemberRow> = sqlx::query_as(&sql)
            .bind(guild_id.as_i64())
            .bind(user_id.as_i64())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        Ok(row.map(Member::from))
    }

    async fn member_profiles(
        &self,
        guild_id: Snowflake,
        limit: u32,
        after: Option<Snowflake>,
    ) -> AppResult<Vec<MemberProfile>> {
        let sql = member_profile_select("WHERE m.guild_id = $1 AND ($2::bigint IS NULL OR m.user_id > $2) ORDER BY m.user_id LIMIT $3");
        let rows: Vec<MemberProfileRow> = sqlx::query_as(&sql)
            .bind(guild_id.as_i64())
            .bind(after.map(|id| id.as_i64()))
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        Ok(rows.into_iter().map(MemberProfile::from).collect())
    }

    async fn members(&self, guild_id: Snowflake) -> AppResult<Vec<Member>> {
        let sql = format!(
            "SELECT {MEMBER_COLUMNS} FROM members m WHERE m.guild_id = $1 ORDER BY m.user_id"
        );
        let rows: Vec<MemberRow> = sqlx::query_as(&sql)
            .bind(guild_id.as_i64())
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        Ok(rows.into_iter().map(Member::from).collect())
    }

    async fn member_ids(&self, guild_id: Snowflake, limit: u32) -> AppResult<Vec<Snowflake>> {
        let rows: Vec<(i64,)> = sqlx::query_as(
            "SELECT user_id FROM members WHERE guild_id = $1 ORDER BY user_id LIMIT $2",
        )
        .bind(guild_id.as_i64())
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows
            .into_iter()
            .map(|(id,)| Snowflake::from_i64(id))
            .collect())
    }

    async fn prune_candidates(
        &self,
        guild_id: Snowflake,
        cutoff: DateTime<Utc>,
        include_roles: &[Snowflake],
    ) -> AppResult<Vec<Snowflake>> {
        let rows: Vec<(i64,)> = sqlx::query_as(
            "SELECT m.user_id FROM members m JOIN guilds g ON g.id = m.guild_id JOIN users u ON u.id = m.user_id              WHERE m.guild_id = $1 AND m.user_id <> g.owner_id AND NOT u.bot AND m.joined_at < $2                AND NOT EXISTS (SELECT 1 FROM sessions s WHERE s.user_id = m.user_id AND s.last_used_at >= $2)                AND NOT EXISTS (SELECT 1 FROM member_roles mr WHERE mr.guild_id = m.guild_id                                AND mr.user_id = m.user_id AND mr.role_id <> ALL($3))              ORDER BY m.user_id",
        )
        .bind(guild_id.as_i64())
        .bind(cutoff)
        .bind(include_roles.iter().map(|id| id.as_i64()).collect::<Vec<_>>())
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows
            .into_iter()
            .map(|(id,)| Snowflake::from_i64(id))
            .collect())
    }

    async fn member_count(&self, guild_id: Snowflake) -> AppResult<u64> {
        let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM members WHERE guild_id = $1")
            .bind(guild_id.as_i64())
            .fetch_one(&self.pool)
            .await
            .map_err(db)?;
        Ok(count as u64)
    }

    async fn ban(&self, guild_id: Snowflake, user_id: Snowflake) -> AppResult<Option<Ban>> {
        let sql = format!(
            "SELECT b.guild_id, b.reason, {user_columns} FROM bans b JOIN users u ON u.id = b.user_id \
             WHERE b.guild_id = $1 AND b.user_id = $2",
            user_columns = prefixed_user_columns()
        );
        let row: Option<BanRow> = sqlx::query_as(&sql)
            .bind(guild_id.as_i64())
            .bind(user_id.as_i64())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        Ok(row.map(Ban::from))
    }

    async fn bans(&self, guild_id: Snowflake) -> AppResult<Vec<Ban>> {
        let sql = format!(
            "SELECT b.guild_id, b.reason, {user_columns} FROM bans b JOIN users u ON u.id = b.user_id \
             WHERE b.guild_id = $1 ORDER BY b.user_id",
            user_columns = prefixed_user_columns()
        );
        let rows: Vec<BanRow> = sqlx::query_as(&sql)
            .bind(guild_id.as_i64())
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        Ok(rows.into_iter().map(Ban::from).collect())
    }

    async fn search(&self, query: Option<&str>, limit: u32) -> AppResult<Vec<Guild>> {
        let sql = format!(
            "SELECT {GUILD_COLUMNS} FROM guilds WHERE ($1::text IS NULL OR name ILIKE $1) ORDER BY id LIMIT $2"
        );
        let rows: Vec<GuildRow> = sqlx::query_as(&sql)
            .bind(query.map(like_pattern))
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        Ok(rows.into_iter().map(Guild::from).collect())
    }

    async fn count(&self) -> AppResult<u64> {
        let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM guilds")
            .fetch_one(&self.pool)
            .await
            .map_err(db)?;
        Ok(count as u64)
    }
    async fn discovery(&self, guild_id: Snowflake) -> AppResult<Option<GuildDiscovery>> {
        let row: Option<(String, Vec<String>, DateTime<Utc>)> = sqlx::query_as(
            "SELECT category, tags, listed_at FROM guild_discovery WHERE guild_id = $1",
        )
        .bind(guild_id.as_i64())
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row.map(|(category, tags, listed_at)| GuildDiscovery {
            category,
            tags,
            listed_at,
        }))
    }

    async fn set_discovery(
        &self,
        guild_id: Snowflake,
        listing: Option<&GuildDiscovery>,
    ) -> AppResult<()> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let community = super::community::lock(&mut tx, guild_id).await?;
        if listing.is_some()
            && !community.is_some_and(|(rules, updates)| rules.is_some() && updates.is_some())
        {
            return Err(application::AppError::invalid(
                "community",
                "COMMUNITY_REQUIRED",
                "Enable Community before listing this server.",
            ));
        }
        match listing {
            Some(listing) => sqlx::query(
                "INSERT INTO guild_discovery (guild_id, category, tags, listed_at) VALUES ($1, $2, $3, $4)                  ON CONFLICT (guild_id) DO UPDATE SET category = EXCLUDED.category, tags = EXCLUDED.tags",
            )
            .bind(guild_id.as_i64())
            .bind(&listing.category)
            .bind(&listing.tags)
            .bind(listing.listed_at)
            .execute(&mut *tx)
            .await
            .map(|_| ()),
            None => sqlx::query("DELETE FROM guild_discovery WHERE guild_id = $1")
                .bind(guild_id.as_i64())
                .execute(&mut *tx)
                .await
                .map(|_| ()),
        }
        .map_err(db)?;
        tx.commit().await.map_err(db)
    }

    async fn has_nsfw_channel(&self, guild_id: Snowflake) -> AppResult<bool> {
        let (found,): (bool,) =
            sqlx::query_as("SELECT EXISTS (SELECT 1 FROM channels WHERE guild_id = $1 AND nsfw)")
                .bind(guild_id.as_i64())
                .fetch_one(&self.pool)
                .await
                .map_err(db)?;
        Ok(found)
    }

    async fn search_discoverable(&self, query: &DiscoveryQuery) -> AppResult<DiscoveryPage> {
        let columns = GUILD_COLUMNS
            .split(", ")
            .map(|c| format!("g.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {columns}, d.category, d.tags, d.listed_at, c.member_count, count(*) OVER () AS total              FROM guild_discovery d              JOIN guilds g ON g.id = d.guild_id              CROSS JOIN LATERAL (SELECT count(*) AS member_count FROM members m WHERE m.guild_id = g.id) c              WHERE c.member_count >= $1                AND g.created_at <= $2                AND g.rules_channel_id IS NOT NULL AND char_length(btrim(g.description)) >= 10                AND NOT EXISTS (SELECT 1 FROM channels ch WHERE ch.guild_id = g.id AND ch.nsfw)                AND ($3::text IS NULL OR d.category = $3)                AND ($4::text IS NULL OR $4 = ANY (d.tags))                AND ($5::text IS NULL OR g.name ILIKE $5 OR g.description ILIKE $5 OR EXISTS (                     SELECT 1 FROM unnest(d.tags) t WHERE t ILIKE $5))              ORDER BY c.member_count DESC, g.id              LIMIT $6 OFFSET $7"
        );
        let rows: Vec<DiscoveryRow> = sqlx::query_as(&sql)
            .bind(query.min_members as i64)
            .bind(query.created_before)
            .bind(query.category.as_deref())
            .bind(query.tag.as_deref())
            .bind(query.text.as_deref().map(like_pattern))
            .bind(i64::from(query.limit))
            .bind(i64::from(query.offset))
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        let total = rows.first().map_or(0, |row| row.total as u64);
        let entries = rows
            .into_iter()
            .map(|row| DiscoveryEntry {
                discovery: GuildDiscovery {
                    category: row.category.clone(),
                    tags: row.tags.clone(),
                    listed_at: row.listed_at,
                },
                member_count: row.member_count as u64,
                guild: row.guild.into(),
            })
            .collect();
        Ok(DiscoveryPage { entries, total })
    }

    async fn template(&self, code: &str) -> AppResult<Option<GuildTemplate>> {
        let sql = format!("SELECT {TEMPLATE_COLUMNS} FROM guild_templates WHERE code = $1");
        let row: Option<TemplateRow> = sqlx::query_as(&sql)
            .bind(code)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        row.map(GuildTemplate::try_from).transpose()
    }

    async fn templates_for_guild(&self, guild_id: Snowflake) -> AppResult<Vec<GuildTemplate>> {
        let sql = format!(
            "SELECT {TEMPLATE_COLUMNS} FROM guild_templates WHERE guild_id = $1 ORDER BY created_at, code"
        );
        let rows: Vec<TemplateRow> = sqlx::query_as(&sql)
            .bind(guild_id.as_i64())
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        rows.into_iter().map(GuildTemplate::try_from).collect()
    }

    async fn insert_template(&self, template: &GuildTemplate) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO guild_templates (code, guild_id, name, description, usage_count, creator_id,              serialized_source_guild, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(&template.code)
        .bind(template.source_guild_id.map(Snowflake::as_i64))
        .bind(&template.name)
        .bind(&template.description)
        .bind(template.usage_count as i32)
        .bind(template.creator_id.map(Snowflake::as_i64))
        .bind(template.source.to_json())
        .bind(template.created_at)
        .bind(template.updated_at)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(db)
    }

    async fn update_template(&self, template: &GuildTemplate) -> AppResult<()> {
        sqlx::query(
            "UPDATE guild_templates SET name = $2, description = $3, serialized_source_guild = $4,              updated_at = $5 WHERE code = $1",
        )
        .bind(&template.code)
        .bind(&template.name)
        .bind(&template.description)
        .bind(template.source.to_json())
        .bind(template.updated_at)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(db)
    }

    async fn delete_template(&self, code: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM guild_templates WHERE code = $1")
            .bind(code)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(db)
    }

    async fn bump_template_usage(&self, code: &str) -> AppResult<()> {
        sqlx::query("UPDATE guild_templates SET usage_count = usage_count + 1 WHERE code = $1")
            .bind(code)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(db)
    }
}

const TEMPLATE_COLUMNS: &str = "code, guild_id, name, description, usage_count, creator_id,      serialized_source_guild, created_at, updated_at";

#[derive(FromRow)]
struct TemplateRow {
    code: String,
    guild_id: i64,
    name: String,
    description: Option<String>,
    usage_count: i32,
    creator_id: Option<i64>,
    serialized_source_guild: serde_json::Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<TemplateRow> for GuildTemplate {
    type Error = application::AppError;

    fn try_from(row: TemplateRow) -> Result<Self, Self::Error> {
        let source = TemplateSource::from_json(&row.serialized_source_guild)
            .map_err(application::AppError::internal)?;
        Ok(GuildTemplate {
            code: row.code,
            name: row.name,
            description: row.description,
            usage_count: row.usage_count.max(0) as u32,
            creator_id: sf_opt(row.creator_id),
            source_guild_id: Some(sf(row.guild_id)),
            source,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

#[derive(FromRow)]
struct DiscoveryRow {
    #[sqlx(flatten)]
    guild: GuildRow,
    category: String,
    tags: Vec<String>,
    listed_at: DateTime<Utc>,
    member_count: i64,
    total: i64,
}

fn prefixed_user_columns() -> String {
    USER_COLUMNS
        .split(", ")
        .map(|column| format!("u.{column}"))
        .collect::<Vec<_>>()
        .join(", ")
}
