//! Field patches keep profile writes independent of security and moderation writes.

use application::{AppError, AppResult, Resource};
use domain::profile::{AppearancePatch, MemberProfilePatch};
use domain::Snowflake;
use sqlx::PgConnection;

use super::error::db;

pub(super) async fn set_user(
    conn: &mut PgConnection,
    user_id: Snowflake,
    patch: &AppearancePatch,
) -> AppResult<()> {
    let colors = patch
        .theme_colors
        .flatten()
        .map(|values| values.map(|v| v as i32).to_vec());
    let result = sqlx::query(
        "UPDATE users SET \
        banner = CASE WHEN $2 THEN $3 ELSE banner END, \
        pronouns = CASE WHEN $4 THEN $5 ELSE pronouns END, \
        theme_colors = CASE WHEN $6 THEN $7 ELSE theme_colors END, \
        accent_color = CASE WHEN $8 THEN $9 ELSE accent_color END WHERE id = $1",
    )
    .bind(user_id.as_i64())
    .bind(patch.banner.is_some())
    .bind(patch.banner.as_ref().and_then(|v| v.as_deref()))
    .bind(patch.pronouns.is_some())
    .bind(patch.pronouns.as_ref().and_then(|v| v.as_deref()))
    .bind(patch.theme_colors.is_some())
    .bind(colors)
    .bind(patch.accent_color.is_some())
    .bind(patch.accent_color.flatten().map(|v| v as i32))
    .execute(conn)
    .await
    .map_err(db)?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(Resource::User));
    }
    Ok(())
}

pub(super) async fn set_member(
    conn: &mut PgConnection,
    guild_id: Snowflake,
    user_id: Snowflake,
    patch: &MemberProfilePatch,
) -> AppResult<()> {
    let result = sqlx::query(
        "UPDATE members SET \
        avatar = CASE WHEN $3 THEN $4 ELSE avatar END, \
        banner = CASE WHEN $5 THEN $6 ELSE banner END, \
        bio = CASE WHEN $7 THEN $8 ELSE bio END WHERE guild_id = $1 AND user_id = $2",
    )
    .bind(guild_id.as_i64())
    .bind(user_id.as_i64())
    .bind(patch.avatar.is_some())
    .bind(patch.avatar.as_ref().and_then(|v| v.as_deref()))
    .bind(patch.banner.is_some())
    .bind(patch.banner.as_ref().and_then(|v| v.as_deref()))
    .bind(patch.bio.is_some())
    .bind(patch.bio.as_ref().and_then(|v| v.as_deref()))
    .execute(conn)
    .await
    .map_err(db)?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(Resource::Member));
    }
    Ok(())
}
