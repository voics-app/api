use domain::session::ClientInfo;
use domain::user::{Credentials, User, UserSettings};
use domain::validation::{self, ValidationErrors};

use crate::context::Ports;
use crate::error::AppResult;
use crate::support::account_mail::send_verification;
use crate::use_cases::auth::tokens::{start_session, AuthTokens};
use crate::write_set::{Mutation, WriteSet};

#[derive(Debug, Clone)]
pub struct RegisterInput {
    pub email: String,
    pub username: String,
    pub password: String,
    pub client: ClientInfo,
}

pub async fn execute(ports: &Ports, input: RegisterInput) -> AppResult<AuthTokens> {
    let email = input.email.trim().to_lowercase();
    let username = input.username.trim().to_owned();

    let mut errors = ValidationErrors::new();
    errors.check(validation::email(&email));
    errors.check(validation::username(&username));
    errors.check(validation::password(&input.password));
    errors.into_result()?;

    let mut taken = ValidationErrors::new();
    if ports.users.find_by_email(&email).await?.is_some() {
        taken.add(
            "email",
            "EMAIL_ALREADY_REGISTERED",
            "Email is already registered.",
        );
    }
    if ports.users.find_by_username(&username).await?.is_some() {
        taken.add(
            "username",
            "USERNAME_ALREADY_TAKEN",
            "Username is unavailable. Try adding numbers or characters.",
        );
    }
    taken.into_result()?;

    let user = User {
        id: ports.ids.next_id(),
        username,
        global_name: None,
        avatar: None,
        bot: false,
        system: false,
        email: Some(email),
        verified: false,
        mfa_enabled: false,
        flags: 0,
        platform_admin: false,
        suspension: None,
        created_at: ports.clock.now(),
    };
    let credentials = Credentials {
        user_id: user.id,
        password_hash: ports.passwords.hash(&input.password).await?,
        totp_secret: None,
        pending_totp_secret: None,
    };
    let mut writes = WriteSet::new();
    writes.mutate(Mutation::InsertUser {
        user: user.clone(),
        credentials: Some(credentials),
        settings: UserSettings::defaults_for(user.id),
    });
    ports.unit_of_work.commit(writes).await?;
    if let Some(address) = &user.email {
        if let Err(error) = send_verification(ports, user.id, &user.username, address, false).await
        {
            tracing::warn!(%error, "verification email not sent at registration");
        }
    }
    start_session(ports, user, None, input.client).await
}
