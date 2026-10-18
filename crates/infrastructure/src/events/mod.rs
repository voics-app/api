//! Domain events mapped onto the P6 envelope, and the outbox relay that
//! publishes them to Redpanda.

pub mod kafka;
pub mod relay;
pub mod route;
