use domain::guild::Member;
use domain::profile::MemberProfilePatch;
use domain::{validation, Snowflake};

use crate::support::images::{store_hashed, ImageUpload};
use crate::{AppError, AppResult, Ports};

pub(super) struct ProfileInput {
    pub actor_id: Snowflake,
    pub avatar: Option<Option<ImageUpload>>,
    pub banner: Option<Option<ImageUpload>>,
    pub bio: Option<Option<String>>,
}

pub(super) async fn apply(
    ports: &Ports,
    member: &mut Member,
    input: ProfileInput,
) -> AppResult<MemberProfilePatch> {
    if input.actor_id != member.user_id
        && (input.avatar.is_some() || input.banner.is_some() || input.bio.is_some())
    {
        return Err(AppError::missing_permissions());
    }
    if let Some(Some(bio)) = &input.bio {
        validation::bio(bio)?;
    }
    let prefix = format!("guilds/{}/users/{}", member.guild_id, member.user_id);
    let patch = MemberProfilePatch {
        avatar: store(ports, &format!("{prefix}/avatars"), input.avatar).await?,
        banner: store(ports, &format!("{prefix}/banners"), input.banner).await?,
        bio: input.bio.map(|value| {
            value
                .map(|bio| bio.trim().to_owned())
                .filter(|bio| !bio.is_empty())
        }),
    };
    patch.apply(member);
    Ok(patch)
}

async fn store(
    ports: &Ports,
    prefix: &str,
    input: Option<Option<ImageUpload>>,
) -> AppResult<Option<Option<String>>> {
    match input {
        Some(Some(image)) => Ok(Some(Some(store_hashed(ports, prefix, image).await?))),
        value => Ok(value.map(|_| None)),
    }
}
