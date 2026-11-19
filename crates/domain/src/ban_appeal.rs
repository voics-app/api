use crate::Snowflake;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq)]
pub struct BanAppeal {
    pub id: Snowflake,
    pub guild_id: Snowflake,
    pub user_id: Snowflake,
    pub ban_created_at: DateTime<Utc>,
    pub question: String,
    pub answer: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolved_by: Option<Snowflake>,
}
