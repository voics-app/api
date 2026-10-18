//! Use cases and ports. Orchestrates the domain; knows nothing about HTTP,
//! SQL or Kafka.

pub mod context;
pub mod error;
pub mod events;
pub mod ports;
pub mod support;
pub mod use_cases;
pub mod write_set;

pub use context::{Actor, ActorKind, Ports, Settings, VoiceRegion};
pub use error::{AppError, AppResult, Denial, Resource};
