// TODO: tighten the public surface of this crate and remove this allow. The
// workspace enforces `unreachable_pub`; coordinator has not yet been audited.
#![allow(unreachable_pub)]

pub mod blockchain_tracker;
pub mod config;
pub mod coordinator;
mod event_processor;
mod flows;
pub mod force_flags;
pub mod monitor;
pub mod store;
mod types;
mod user_requests;

pub const RUNTIME_ENV_LOCAL: &str = "local";
