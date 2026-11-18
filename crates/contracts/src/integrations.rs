//! Public bot-install consent records, deliberately separate from execution credentials.
use crate::{
    objects::ApplicationDto,
    wire::{Id, Perm},
};
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Serialize)]
pub struct AuthorizationGuildDto {
    pub id: Id,
    pub name: String,
    pub icon: Option<String>,
}

#[derive(Serialize)]
pub struct ApplicationAuthorizationDto {
    pub id: Id,
    pub application: ApplicationDto,
    pub guild: AuthorizationGuildDto,
    pub scopes: Vec<String>,
    pub permissions: Perm,
    pub created_at: DateTime<Utc>,
    pub authorized_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct GuildIntegrationDto {
    pub application: ApplicationDto,
    pub permissions: Perm,
    pub installed_at: DateTime<Utc>,
}
