//! Path segment parsing shared by the resource routers.

use application::{AppError, Resource};
use domain::Snowflake;

use crate::error::ApiError;

/// A malformed id can never name a resource, so it is reported as unknown.
pub fn snowflake(raw: &str, resource: Resource) -> Result<Snowflake, ApiError> {
    raw.parse().map_err(|_| AppError::NotFound(resource).into())
}
