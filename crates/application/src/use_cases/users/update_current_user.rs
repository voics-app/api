use domain::user::User;
use domain::validation::{self, ValidationErrors};

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Denial};
use crate::events::DomainEvent;
use crate::support::account_mail;
use crate::support::credentials::confirm_password;
use crate::support::images::{store_hashed, ImageUpload};
use crate::support::sessions;
use crate::support::views::require_user;
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone, Default)]
pub struct UpdateCurrentUserInput {
    pub username: Option<String>,
    /// `Some(None)` clears the display name.
    pub global_name: Option<Option<String>>,
    /// `Some(None)` clears the "About me" text.
    pub bio: Option<Option<String>>,
    /// `Some(None)` removes the avatar.
    pub avatar: Option<Option<ImageUpload>>,
    /// The current password; required with a new username, email or password.
    pub password: Option<String>,
    pub new_password: Option<String>,
    /// Takes effect once the new address is verified.
    pub email: Option<String>,
}

/// What a sensitive change needs after the profile write succeeded.
struct Aftermath {
    verify_email: Option<String>,
    password_changed: bool,
}

pub async fn execute(
    ports: &Ports,
    actor: &Actor,
    input: UpdateCurrentUserInput,
) -> AppResult<User> {
    let mut user = require_user(ports, actor.user_id).await?;
    let new_email = input.email.as_deref().map(|e| e.trim().to_lowercase());
    validate(&input, new_email.as_deref())?;

    let username_change = input
        .username
        .clone()
        .filter(|username| *username != user.username);
    let email_change = new_email.filter(|email| user.email.as_deref() != Some(email.as_str()));
    let sensitive =
        username_change.is_some() || email_change.is_some() || input.new_password.is_some();
    if sensitive {
        if actor.is_bot() {
            return Err(AppError::Forbidden(Denial::BotsNotAllowed));
        }
        let password = input.password.as_deref().ok_or_else(|| {
            AppError::invalid(
                "password",
                "PASSWORD_REQUIRED",
                "Your current password is required for this change.",
            )
        })?;
        confirm_password(ports, user.id, password).await?;
    }

    if let Some(username) = username_change {
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
    if let Some(email) = &email_change {
        if ports.users.find_by_email(email).await?.is_some() {
            return Err(ValidationErrors::single(
                "email",
                "EMAIL_ALREADY_REGISTERED",
                "Email is already registered.",
            )
            .into());
        }
    }
    if let Some(global_name) = input.global_name {
        user.global_name = global_name.map(|n| n.trim().to_owned());
    }
    if let Some(bio) = input.bio {
        user.bio = bio.map(|b| b.trim().to_owned()).filter(|b| !b.is_empty());
    }
    if let Some(avatar) = input.avatar {
        user.avatar = match avatar {
            Some(image) => Some(store_hashed(ports, &format!("avatars/{}", user.id), image).await?),
            None => None,
        };
    }
    let mut writes = WriteSet::new();
    if let Some(new_password) = &input.new_password {
        writes.mutate(Mutation::SetPasswordHash {
            user_id: user.id,
            hash: ports.passwords.hash(new_password).await?,
        });
    }
    writes
        .mutate(Mutation::UpdateUser(user.clone()))
        .emit(DomainEvent::UserUpdate(user.clone()));
    ports.unit_of_work.commit(writes).await?;

    after(
        ports,
        actor,
        &user,
        Aftermath {
            verify_email: email_change,
            password_changed: input.new_password.is_some(),
        },
    )
    .await?;
    Ok(user)
}

fn validate(input: &UpdateCurrentUserInput, email: Option<&str>) -> Result<(), ValidationErrors> {
    let mut errors = ValidationErrors::new();
    if let Some(username) = &input.username {
        errors.check(validation::username(username));
    }
    if let Some(Some(name)) = &input.global_name {
        errors.check(validation::global_name(name));
    }
    if let Some(Some(bio)) = &input.bio {
        errors.check(validation::bio(bio));
    }
    if let Some(email) = email {
        errors.check(validation::email(email));
    }
    if let Some(password) = &input.new_password {
        if let Err(error) = validation::password(password) {
            errors.add("new_password", error.code, error.message);
        }
    }
    errors.into_result()
}

/// Mail and session consequences of a committed change. A password change signs
/// every other session out (the caller's stays); an email change mails the new
/// address its token and tells the old one.
async fn after(ports: &Ports, actor: &Actor, user: &User, aftermath: Aftermath) -> AppResult<()> {
    if let Some(new_address) = &aftermath.verify_email {
        if let Err(error) =
            account_mail::send_verification(ports, user.id, &user.username, new_address, true).await
        {
            tracing::warn!(%error, "verification email not sent for email change");
        }
        if let Some(old_address) = &user.email {
            account_mail::notify(
                ports,
                old_address,
                &format!("Your {} email is changing", account_mail::brand(ports)),
                &format!(
                    "A change of the email address of your {} account was requested.",
                    account_mail::brand(ports)
                ),
            )
            .await;
        }
    }
    if aftermath.password_changed {
        sessions::revoke(ports, user.id, actor.session_id()).await?;
        if let Some(address) = &user.email {
            account_mail::notify(
                ports,
                address,
                &format!("Your {} password was changed", account_mail::brand(ports)),
                &format!(
                    "The password of your {} account was changed and your other sessions were signed out.",
                    account_mail::brand(ports)
                ),
            )
            .await;
        }
    }
    Ok(())
}
