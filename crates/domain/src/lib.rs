//! Discordy core domain: entities, value objects and pure business rules.
//! No framework or IO dependencies.

pub mod access;
pub mod account;
pub mod application;
pub mod audit;
pub mod channel;
pub mod delivery_mode;
pub mod device;
pub mod discovery;
pub mod emoji;
pub mod guild;
pub mod invite;
pub mod mfa;
pub mod notification;
pub mod permissions;
pub mod relationship;
pub mod snowflake;
pub mod template;
pub mod user;
pub mod user_preferences;
pub mod validation;

pub use snowflake::Snowflake;
