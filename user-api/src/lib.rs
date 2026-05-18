// user-api is a thin wrapper layer, not the production peg pipeline — pedantic
// clippy lints are not enforced here.
#![allow(clippy::pedantic)]

pub mod config;
pub mod errors;
pub mod server;
pub mod sync_contracts_gateway;

pub use server::Server;
