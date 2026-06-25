#![cfg_attr(not(test), forbid(unsafe_code))]

pub mod blockchain_tracker;
pub mod config;
pub mod coordinator;
mod event_processor;
mod flows;
pub mod force_flags;
pub mod monitor;
pub mod store;
#[cfg(test)]
mod test_metrics;
mod types;
mod user_requests;

// Runtime tier classifications, grouped by Rootstock backend (not operator location).
// Both cargo-mode and docker-mode operators against the same backend report the same tier.
pub const RUNTIME_ENV_LOCAL_ANVIL: &str = "local-anvil";
pub const RUNTIME_ENV_LOCAL_RSKJ: &str = "local-rskj";
