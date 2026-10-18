use domain::user::User;
use domain::validation::{self, ValidationErrors};

use crate::context::{Actor, Ports};
use crate::error::AppResult;
use crate::events::DomainEvent;
use crate::support::images::{store_hashed, ImageUpload};
use crate::support::views::require_user;
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone, Default)]
pub struct UpdateCurrentUserInput {
    pub username: Option<String>,
    /// `Some(None)` clears the display name.
    pub global_name: Option<Option<String>>,
    /// `Some(None)` removes the avatar.
    pub avatar: Option<Option<ImageUpload>>,
}

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    input: UpdateCurrentUserInput,
) -> AppResult<User> {
    let mut user = require_user(ports, actor.user_id).await?;
    let mut errors = ValidationErrors::new();
    if let Some(username) = &input.username {
        errors.check(validation::username(username));
    }
    if let Some(Some(name)) = &input.global_name {
        errors.check(validation::global_name(name));
    }
    errors.into_result()?;

    if let Some(username) = input.username.filter(|u| *u != user.username) {
        if ports.users.find_by_username(&username).await?.is_some() {
            return Err(ValidationErrors::single(
                "username",
                "USERNAME_ALREADY_TAKEN",
                "Username is unavailable.",
            )
            .into());
        }
        user.username = username;
    }
    if let Some(global_name) = input.global_name {
        user.global_name = global_name.map(|n| n.trim().to_owned());
    }
    if let Some(avatar) = input.avatar {
        user.avatar = match avatar {
            Some(image) => Some(store_hashed(ports, &format!("avatars/{}", user.id), image).await?),
            None => None,
        };
    }
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpdateUser(user.clone()))
        .emit(DomainEvent::UserUpdate(user.clone()));
    ports.unit_of_work.commit(writes).await?;
    Ok(user)
}
