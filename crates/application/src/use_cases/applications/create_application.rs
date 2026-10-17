use domain::application::Application;
use domain::user::{User, UserSettings};
use domain::validation;

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult};
use crate::use_cases::applications::app_access::{view, ApplicationView};
use crate::write_set::{Mutation, WriteSet};

pub const MAX_APPLICATIONS_PER_OWNER: usize = 25;

#[derive(Debug, Clone, Default)]
pub struct CreateApplicationInput {
    pub name: String,
    pub description: Option<String>,
}

/// Lowercase username derived from the application name.
fn bot_username_base(name: &str) -> String {
    let cleaned: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '.' { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim_matches(|c| c == '_' || c == '.').replace("..", ".");
    let base: String = trimmed.chars().take(24).collect();
    if base.chars().count() < 2 {
        "bot".to_owned()
    } else {
        base
    }
}

/// Creates an application together with its bot user.
pub async fn execute(ports: &Ports, actor: &Actor, input: CreateApplicationInput) -> AppResult<ApplicationView> {
    actor.require_user()?;
    validation::application_name(&input.name)?;
    let description = input.description.unwrap_or_default();
    if description.chars().count() > 400 {
        return Err(AppError::invalid("description", "BASE_TYPE_BAD_LENGTH", "Must be at most 400 characters."));
    }
    if ports.applications.list_for_owner(actor.user_id).await?.len() >= MAX_APPLICATIONS_PER_OWNER {
        return Err(AppError::BadRequest { code: 30000, message: "Maximum number of applications reached.".into() });
    }
    let now = ports.clock.now();
    let application_id = ports.ids.next_id();
    let bot_id = ports.ids.next_id();
    let base = bot_username_base(&input.name);
    let username = if ports.users.find_by_username(&base).await?.is_some() {
        let suffix = bot_id.value() % 1_000_000;
        format!("{base}{suffix}")
    } else {
        base
    };
    let bot = User {
        id: bot_id,
        username,
        global_name: Some(input.name.trim().to_owned()),
        avatar: None,
        bot: true,
        system: false,
        email: None,
        verified: true,
        mfa_enabled: false,
        flags: 0,
        platform_admin: false,
        suspension: None,
        created_at: now,
    };
    let application = Application {
        id: application_id,
        name: input.name.trim().to_owned(),
        description,
        icon: None,
        owner_id: actor.user_id,
        bot_user_id: bot_id,
        bot_public: true,
        bot_require_code_grant: false,
        verify_key: ports.secrets.verify_key(),
        flags: 0,
        interactions_endpoint_url: None,
        created_at: now,
    };
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::InsertUser { user: bot.clone(), credentials: None, settings: UserSettings::defaults_for(bot_id) })
        .mutate(Mutation::InsertApplication(application.clone()));
    ports.unit_of_work.commit(writes).await?;
    view(ports, application).await
}

#[cfg(test)]
mod tests {
    use super::bot_username_base;

    #[test]
    fn derives_valid_bot_usernames() {
        assert_eq!(bot_username_base("Moderation Bot!"), "moderation_bot");
        assert_eq!(bot_username_base("!!"), "bot");
        assert_eq!(bot_username_base("a..b"), "a.b");
        assert_eq!(bot_username_base(&"x".repeat(40)).len(), 24);
    }
}
