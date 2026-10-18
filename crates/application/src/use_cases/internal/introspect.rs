use uuid::Uuid;

use domain::Snowflake;

use crate::context::{ActorKind, Ports};
use crate::error::AppResult;
use crate::use_cases::auth::authenticate::{self, Credential};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    User,
    Bot,
}

impl TokenKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            TokenKind::User => "user",
            TokenKind::Bot => "bot",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Introspection {
    pub user_id: Snowflake,
    pub kind: TokenKind,
    pub application_id: Option<Snowflake>,
    pub session_id: Option<Uuid>,
}

/// `POST /internal/v1/auth/introspect`. Accepts a user JWT or a bot token,
/// with or without the `Bearer `/`Bot ` prefix. JWTs have two dots; bot
/// tokens (`<base64url id>.<secret>`) have one.
pub async fn execute(ports: &Ports, token: &str) -> AppResult<Introspection> {
    let token = token.trim();
    let credential = match token.strip_prefix("Bot ") {
        Some(bot) => Credential::Bot(bot.trim()),
        None if token.starts_with("Bearer ") => Credential::parse(token),
        None if token.matches('.').count() == 2 => Credential::Bearer(token),
        None => Credential::Bot(token),
    };
    let actor = authenticate::execute(ports, credential).await?;
    Ok(match actor.kind {
        ActorKind::User { session_id, .. } => Introspection {
            user_id: actor.user_id,
            kind: TokenKind::User,
            application_id: None,
            session_id: Some(session_id),
        },
        ActorKind::Bot { application_id } => Introspection {
            user_id: actor.user_id,
            kind: TokenKind::Bot,
            application_id: Some(application_id),
            session_id: None,
        },
    })
}
