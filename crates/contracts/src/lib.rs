//! Wire shapes pinned by the master plan (P3, P4, P6, P9).
//!
//! Everything that crosses a process boundary is defined here and nowhere
//! else, so this module can be swapped for the generated `discordy-contracts`
//! crate once it is published.

pub mod envelope;
pub mod errors;
pub mod internal;
pub mod objects;
pub mod requests;
pub mod wire;

pub use wire::{Id, Perm};
