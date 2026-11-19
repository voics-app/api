use crate::AppResult;
use async_trait::async_trait;
use domain::Snowflake;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub provider: String,
    pub account_id: String,
    pub name: String,
    pub visible: bool,
}

#[async_trait]
pub trait Connections: Send + Sync {
    async fn start(&self, user: Snowflake, session: Uuid) -> AppResult<String>;
    async fn callback(&self, state: &str, code: &str) -> AppResult<()>;
    async fn cancel(&self, state: &str) -> AppResult<()>;
    async fn list(&self, user: Snowflake, public_only: bool) -> AppResult<Vec<Connection>>;
    async fn visibility(&self, user: Snowflake, visible: bool) -> AppResult<()>;
    async fn delete(&self, user: Snowflake) -> AppResult<()>;
}
