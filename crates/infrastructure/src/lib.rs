//! Adapters implementing the application ports: Postgres, the outbox relay
//! to Redpanda, argon2, Ed25519 JWTs, Valkey, MinIO and the messages service.

pub mod events;
pub mod postgres;
